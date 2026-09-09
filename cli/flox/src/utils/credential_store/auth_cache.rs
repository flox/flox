//! A non-secret note of what the last credential read found.
//!
//! Reading the OS keyring is the most expensive thing an invocation does
//! before it knows what it was asked to do: a macOS Keychain read measures
//! ~9.5 ms, most of the ~12 ms it takes the process to start at all. Yet the
//! only questions the startup path asks of the credential are "is one
//! stored", "has it expired", and "which pseudonymous subject metrics are
//! attributed to".
//!
//! Recording the summary beside the credential turns those questions into a
//! ~2 µs file read and leaves the keyring for the invocations that actually
//! send a FloxHub request. See [super::CredentialStores::resolve].
//!
//! The record also carries the user's handle, which for an opaque token costs
//! a `/me` round trip rather than a keyring read. That half is keyed the same
//! way — by FloxHub instance — because the point is to answer before the
//! credential has been loaded, and keying on the secret would gate the cheap
//! fact behind the costly one. A caller that *does* hold the secret checks the
//! recorded `fingerprint` against it before believing the handle; see
//! [AuthContextStorageExt::seed_from].
//! Identity records cover both keyring and plaintext credentials. Only a
//! keyring record may defer a keyring read; a plaintext record supplies identity
//! only after its fingerprint matches the token loaded from configuration.
//!
//! **Nothing here is secret, and nothing secret may be added.** Writing the
//! token to a plain file would undo the reason it lives in the keyring at all.
//! The subject claim and the handle identify a user across sessions, so the
//! file is written `0600` like the plain-text credential, and the fingerprint
//! is a blake3 digest that names the credential without revealing it.
//!
//! The record is a claim about a credential this invocation has not read. It
//! goes stale whenever the keychain changes behind our back — another `flox`
//! version, another machine syncing the keychain, Keychain Access, a direct
//! `security` invocation. The blast radius is bounded: a missing or spurious
//! "not logged in" reminder, and one telemetry event attributed to the wrong
//! subject. A recorded handle is also advisory until the credential is loaded;
//! identity lookups check its fingerprint before reusing it. The first invocation
//! that actually needs the token reads the keyring and rewrites the record, so a
//! stale one does not survive use.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flox_rust_sdk::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use floxhub_client::auth::storage::{AuthContextStorageExt, CachedFacts};
use floxhub_client::{AuthContext, CredentialKind};
use serde::{Deserialize, Serialize};
use tracing::debug;
use url::Url;

use super::TokenStorage;

/// Bumped when the on-disk shape changes. A record written by a different
/// version is ignored rather than migrated — the cost of a miss is one
/// keyring read.
const AUTH_STATE_VERSION: u32 = 1;

/// The on-disk form of a [CachedFacts].
///
/// `expires_at` is stored as a Unix timestamp rather than a formatted date so
/// the file does not depend on chrono's serde feature, and `account` is
/// repeated inside the file so a record can be rejected when it does not
/// belong to the FloxHub instance being asked about — the filename is derived
/// from the URL and is not injective.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AuthStateRecord {
    version: u32,
    account: String,
    storage: TokenStorage,
    logged_in: bool,
    kind: CredentialKind,
    requires_login: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    handle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint: Option<String>,
}

/// The recorded summary of the credential stored for one FloxHub instance.
#[derive(Debug, Clone)]
pub(super) struct AuthCache {
    path: PathBuf,
    account: String,
}

impl AuthCache {
    /// The record for `account`, kept in `cache_dir`.
    ///
    /// The cache dir rather than the runtime dir: the record describes a
    /// credential that outlives the login session, so tying the record to the
    /// session would discard it while the thing it describes is still there.
    /// It relies on being rewritten for invalidation instead — see the module
    /// docs.
    pub(super) fn new(cache_dir: impl AsRef<Path>, account: &Url) -> Self {
        let account = account.as_str().to_string();
        Self {
            path: cache_dir
                .as_ref()
                .join(format!("auth-state-{}.json", filename_slug(&account))),
            account,
        }
    }

    /// The recorded summary, or `None` when there is nothing usable to read.
    ///
    /// Every failure — no file yet, a truncated write, a record from another
    /// version or another FloxHub instance — is a miss, and a miss simply
    /// costs the keyring read this record exists to avoid.
    fn read(&self) -> Option<AuthStateRecord> {
        let contents = match fs::read_to_string(&self.path) {
            Ok(contents) => contents,
            Err(err) => {
                debug!(path = ?self.path, error = %err, "no recorded auth state");
                return None;
            },
        };
        let record: AuthStateRecord = match serde_json::from_str(&contents) {
            Ok(record) => record,
            Err(err) => {
                debug!(path = ?self.path, error = %err, "could not parse the recorded auth state");
                return None;
            },
        };
        if record.version != AUTH_STATE_VERSION || record.account != self.account {
            debug!(
                path = ?self.path,
                version = record.version,
                "recorded auth state does not describe this credential"
            );
            return None;
        }
        Some(record)
    }

    /// Defer a keyring read only when the record describes keyring state.
    /// A plaintext record cannot establish whether a keyring credential exists.
    pub(super) fn defer_or_resolve(
        &self,
        resolve: impl Fn() -> AuthContext + Send + Sync + 'static,
    ) -> AuthContext {
        let context = match self
            .read()
            .filter(|record| record.storage == TokenStorage::Keyring)
        {
            Some(record) => AuthContext::deferred(record.into(), resolve),
            None => resolve(),
        };
        self.configure(context, TokenStorage::Keyring)
    }

    /// The recorded facts, or `None` on a miss.
    fn recorded_facts(&self) -> Option<CachedFacts> {
        self.read().map(Into::into)
    }

    /// Attach this record to a credential and persist successful identity lookups.
    /// Environment overrides may read a matching record but never overwrite it.
    pub(super) fn configure(&self, context: AuthContext, storage: TokenStorage) -> AuthContext {
        if let Some(facts) = self.recorded_facts() {
            context.seed_from(&facts);
        }
        if std::env::var(FLOXHUB_TOKEN_ENV_VAR).is_ok() {
            return context;
        }
        let cache = self.clone();
        context.with_identity_recorder(move |facts| cache.write_facts(facts, storage))
    }

    /// Record a credential's non-secret properties.
    ///
    /// Best effort: a command must not fail because a cache could not be
    /// written. The write goes to a sibling temp file and is renamed into
    /// place so a concurrent reader sees either the old record or the new one.
    pub(super) fn write(&self, context: &AuthContext, storage: TokenStorage) {
        self.write_facts(&context.cached_facts(), storage);
    }

    fn write_facts(&self, facts: &CachedFacts, storage: TokenStorage) {
        if let Err(err) = self.try_write(facts, storage) {
            debug!(path = ?self.path, error = %err, "could not record auth state");
        }
    }

    /// Drop the record, at logout or whenever the credential's fate is
    /// unknown. Best effort, and a missing record is already the state we
    /// want.
    pub(super) fn remove(&self) {
        match fs::remove_file(&self.path) {
            Ok(()) => {},
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {},
            Err(err) => {
                debug!(path = ?self.path, error = %err, "could not drop recorded auth state")
            },
        }
    }

    fn try_write(&self, facts: &CachedFacts, storage: TokenStorage) -> Result<(), anyhow::Error> {
        let record = AuthStateRecord {
            version: AUTH_STATE_VERSION,
            account: self.account.clone(),
            storage,
            logged_in: facts.logged_in,
            kind: facts.kind,
            requires_login: facts.requires_login,
            expires_at: facts.expires_at.map(|expiry| expiry.timestamp()),
            subject: facts.subject.clone(),
            handle: facts.handle.clone(),
            fingerprint: facts.fingerprint.clone(),
        };
        let contents = serde_json::to_string(&record)?;

        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // A distinct temp name per process: two invocations recording at once
        // must not write into the same file and rename a half-written record
        // into place.
        let temp_path = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        fs::write(&temp_path, contents)?;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o600))?;
        fs::rename(&temp_path, &self.path)?;
        Ok(())
    }
}

impl From<AuthStateRecord> for CachedFacts {
    fn from(record: AuthStateRecord) -> Self {
        Self {
            logged_in: record.logged_in,
            kind: record.kind,
            requires_login: record.requires_login,
            expires_at: record
                .expires_at
                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0)),
            subject: record.subject,
            handle: record.handle,
            fingerprint: record.fingerprint,
        }
    }
}

/// A filename-safe rendering of a FloxHub URL, e.g. `https-hub.flox.dev`.
///
/// Readable rather than hashed: there is nothing secret about which FloxHub
/// instance a user talks to, and a name that says which record is which is
/// worth more than injectivity. Two URLs can collide here; [AuthCache::read]
/// rejects a record whose `account` does not match.
fn filename_slug(account: &str) -> String {
    let mut slug = String::with_capacity(account.len());
    for character in account.chars() {
        match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' => slug.push(character),
            // Collapse runs of separators, so `https://host` reads as
            // `https-host` rather than `https---host`.
            _ if !slug.ends_with('-') => slug.push('-'),
            _ => {},
        }
    }
    slug.trim_matches('-').to_string()
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use floxhub_client::auth::storage::credential_fingerprint;
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::*;

    fn account() -> Url {
        Url::parse("https://hub.flox.dev").unwrap()
    }

    fn facts() -> CachedFacts {
        CachedFacts {
            logged_in: true,
            kind: CredentialKind::Auth0,
            requires_login: true,
            expires_at: Some(
                DateTime::<Utc>::from_timestamp(1_788_000_000, 0).expect("valid timestamp"),
            ),
            subject: Some("auth0|cache-round-trip".to_string()),
            handle: Some("testuser".to_string()),
            fingerprint: Some(credential_fingerprint("cache-round-trip-secret")),
        }
    }

    fn context() -> AuthContext {
        AuthContext::deferred(facts(), || {
            panic!("cache tests must not resolve the credential")
        })
    }

    #[test]
    fn records_and_reads_back_auth_state() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());

        cache.write(&context(), TokenStorage::Keyring);

        assert_eq!(
            cache.read(),
            Some(AuthStateRecord {
                version: AUTH_STATE_VERSION,
                account: account().to_string(),
                storage: TokenStorage::Keyring,
                logged_in: true,
                kind: CredentialKind::Auth0,
                requires_login: true,
                expires_at: Some(1_788_000_000),
                subject: Some("auth0|cache-round-trip".to_string()),
                handle: Some("testuser".to_string()),
                fingerprint: Some(credential_fingerprint("cache-round-trip-secret")),
            })
        );
    }

    /// The record names the credential without carrying it: the fingerprint
    /// is the whole point, and the secret must never reach the file.
    #[test]
    fn the_record_names_the_credential_without_carrying_it() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        let secret = "flox_pat_never-on-disk";

        cache.write(
            &AuthContext::new_from_token(Some(secret)),
            TokenStorage::Keyring,
        );

        let contents = fs::read_to_string(&cache.path).unwrap();
        assert!(
            !contents.contains(secret),
            "the record carried the secret: {contents}"
        );
        assert!(
            contents.contains(&credential_fingerprint(secret)),
            "the record should name the credential by fingerprint: {contents}"
        );
    }

    /// The seeding path end to end: a handle recorded for one credential is
    /// adopted by that credential and by no other.
    #[test]
    fn seeding_adopts_a_handle_recorded_for_the_same_credential() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        let secret = "flox_pat_cache-seed-test";
        cache.write(
            &AuthContext::deferred(
                CachedFacts {
                    handle: Some("seeded-user".to_string()),
                    fingerprint: Some(credential_fingerprint(secret)),
                    ..CachedFacts::default()
                },
                || panic!("cache tests must not resolve the credential"),
            ),
            TokenStorage::Keyring,
        );

        let same = AuthContext::new_from_token(Some(secret));
        let other = AuthContext::new_from_token(Some("flox_pat_cache-seed-test-other"));
        let same = cache.configure(same, TokenStorage::Plaintext);
        let other = cache.configure(other, TokenStorage::Plaintext);

        assert_eq!(same.handle(), Some("seeded-user".to_string()));
        assert_eq!(
            other.handle(),
            None,
            "a record for a different credential must be ignored"
        );
    }

    /// The facts survive the round trip through the file, not just the
    /// on-disk shape: this is what the startup path actually reads back.
    #[test]
    fn deferring_from_a_record_reproduces_the_facts() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Keyring);

        let deferred =
            cache.defer_or_resolve(|| panic!("a valid record must not resolve the credential"));

        assert_eq!(deferred.cached_facts(), facts());
    }

    #[test]
    fn plaintext_identity_does_not_stand_in_for_a_keyring_credential() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Plaintext);

        let context = cache.defer_or_resolve(AuthContext::default);

        assert_eq!(
            context.cached_facts(),
            AuthContext::default().cached_facts()
        );
    }

    #[test]
    fn plaintext_identity_is_reused_for_the_matching_configured_token() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        let credential = AuthContext::new_from_token(Some("flox_pat_plaintext-reuse"));
        let facts = CachedFacts {
            handle: Some("plaintext-user".into()),
            subject: Some("account|plaintext".into()),
            ..credential.cached_facts()
        };
        cache.write(
            &AuthContext::deferred(facts.clone(), || {
                panic!("recording must not load the credential")
            }),
            TokenStorage::Plaintext,
        );

        let context = cache.configure(credential, TokenStorage::Plaintext);

        assert_eq!(context.handle(), Some("plaintext-user".into()));
        assert_eq!(context.cached_facts(), facts);
    }

    #[test]
    fn a_record_without_storage_provenance_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Keyring);
        let mut record: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&cache.path).unwrap()).unwrap();
        record.as_object_mut().unwrap().remove("storage");
        fs::write(&cache.path, serde_json::to_string(&record).unwrap()).unwrap();

        assert_eq!(cache.read(), None);
    }

    #[test]
    fn no_record_reads_as_a_miss() {
        let dir = TempDir::new().unwrap();

        assert_eq!(AuthCache::new(dir.path(), &account()).read(), None);
    }

    /// The record is written before the command that could leak it exists, so
    /// the mode is checked rather than assumed.
    #[test]
    fn record_is_owner_only() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());

        cache.write(&context(), TokenStorage::Keyring);

        let mode = fs::metadata(&cache.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    /// The filename is derived from the URL and is not injective — a trailing
    /// path separator is one way two instances land on the same name — so the
    /// account is checked against the record's own copy.
    #[test]
    fn record_for_another_floxhub_sharing_a_filename_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let hub = Url::parse("https://flox.example/hub").unwrap();
        let sibling = Url::parse("https://flox.example/hub/").unwrap();
        AuthCache::new(dir.path(), &hub).write(&context(), TokenStorage::Keyring);

        let colliding = AuthCache::new(dir.path(), &sibling);
        assert_eq!(
            colliding.path,
            AuthCache::new(dir.path(), &hub).path,
            "the two URLs must actually share a filename for this to test anything"
        );

        assert_eq!(colliding.read(), None);
    }

    #[test]
    fn record_from_another_version_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Keyring);

        let contents = fs::read_to_string(&cache.path).unwrap();
        fs::write(
            &cache.path,
            contents.replace(
                &format!("\"version\":{AUTH_STATE_VERSION}"),
                "\"version\":999",
            ),
        )
        .unwrap();

        assert_eq!(cache.read(), None);
    }

    #[test]
    fn a_truncated_record_is_a_miss() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Keyring);
        let contents = fs::read_to_string(&cache.path).unwrap();
        fs::write(&cache.path, &contents[..contents.len() / 2]).unwrap();

        assert_eq!(cache.read(), None);
    }

    #[test]
    fn removing_a_record_leaves_a_miss_and_removing_nothing_is_fine() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(&context(), TokenStorage::Keyring);

        cache.remove();
        assert_eq!(cache.read(), None);

        cache.remove();
        assert_eq!(cache.read(), None);
    }

    /// `FLOX_FLOXHUB_TOKEN` is a one-invocation override, so recording what
    /// it resolved would leave the record describing a credential the next
    /// invocation will not have.
    #[test]
    fn an_env_credential_is_not_recorded() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        let secret = floxhub_client::auth::test_helpers::FAKE_TOKEN;

        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, Some(secret), || {
            let context = cache.configure(
                AuthContext::new_from_token(Some(secret)),
                TokenStorage::Plaintext,
            );
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let client = floxhub_client::client::test_helpers::new_noop();
                context.identity(&client).await.unwrap().unwrap();
            });
        });

        assert_eq!(cache.read(), None);
    }

    /// Every other source persists, so the record describes what a later
    /// invocation will actually find. The rule is about lifetime, not about
    /// which backend holds the credential.
    #[test]
    fn a_stored_credential_is_recorded() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        let secret = floxhub_client::auth::test_helpers::FAKE_TOKEN;

        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let context = cache.configure(
                AuthContext::new_from_token(Some(secret)),
                TokenStorage::Plaintext,
            );
            tokio::runtime::Runtime::new().unwrap().block_on(async {
                let client = floxhub_client::client::test_helpers::new_noop();
                context.identity(&client).await.unwrap().unwrap();
            });
        });

        assert_eq!(
            cache.read().and_then(|record| record.fingerprint),
            Some(credential_fingerprint(secret))
        );
    }

    /// Expiry is stored as a timestamp and evaluated on read, so a record
    /// written while the credential was valid reports it as expired once the
    /// moment passes.
    #[test]
    fn an_expired_record_reads_as_unauthenticated() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(
            &AuthContext::deferred(
                CachedFacts {
                    expires_at: Some(Utc::now() - TimeDelta::hours(1)),
                    ..facts()
                },
                || panic!("cache tests must not resolve the credential"),
            ),
            TokenStorage::Keyring,
        );

        let deferred = cache.defer_or_resolve(|| panic!("a valid record must be a cache hit"));
        assert!(deferred.is_unauthenticated());
    }

    /// A credential that is not login-gated — Kerberos, or an opaque token —
    /// records `logged_in: false` yet must not read back as unauthenticated.
    #[test]
    fn a_record_without_login_gating_is_not_unauthenticated() {
        let dir = TempDir::new().unwrap();
        let cache = AuthCache::new(dir.path(), &account());
        cache.write(
            &floxhub_client::AuthContext::from_kerberos(None),
            TokenStorage::Keyring,
        );

        let deferred = cache.defer_or_resolve(|| panic!("a valid record must be a cache hit"));
        assert!(!deferred.is_unauthenticated());
    }
}
