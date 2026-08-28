//! Prepare and persist authentication for one FloxHub instance.
//!
//! [CredentialStores] selects credentials, migrates storage, and coordinates
//! the private non-secret cache. Consumers receive a ready-to-use [AuthContext];
//! they do not need to coordinate deferred loading or cache updates.
//!
//! Secret backends live in [keyring], [plaintext], and [mock].

mod auth_cache;
mod keyring;
mod mock;
mod plaintext;
/// macOS-only in production; compiled under `test` everywhere so the backend
/// is exercised against a recording fake on Linux CI.
#[cfg(any(target_os = "macos", test))]
mod security_cli;

use std::path::{Path, PathBuf};

use auth_cache::AuthCache;
use enum_dispatch::enum_dispatch;
use flox_config::{Config, FLOX_CONFIG_FILE, TokenStorageMode};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use floxhub_client::AuthContext;
use indoc::indoc;
use keyring::KeyringStore;
use mock::MockStore;
use plaintext::PlaintextStore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use crate::utils::message;

/// Errors from credential storage operations.
///
/// Per the project conventions, credential redaction belongs here rather than
/// at call sites: the underlying writes (`update_config`) never interpolate
/// the token into their messages, and no variant carries the secret.
#[derive(Debug, Error)]
pub enum CredentialStoreError {
    /// A read or write against the plaintext `flox.toml` failed.
    #[error("could not access the plaintext credential file")]
    Plaintext(#[source] anyhow::Error),

    /// Failed to set owner-only permissions on the plaintext credential file.
    #[error("could not set permissions on the plaintext credential file")]
    SetPermissions(#[source] std::io::Error),

    /// Could not read the plaintext credential file to probe its contents.
    #[error("could not read the plaintext credential file")]
    ReadPlaintext(#[source] std::io::Error),

    /// Could not parse the plaintext credential file as TOML.
    #[error("could not parse the plaintext credential file")]
    ParsePlaintext(#[source] toml_edit::TomlError),

    /// An OS keyring failure. `NoDefaultStore`, `PlatformFailure`, and
    /// `NoStorageAccess` mean no usable backend is available (logout treats
    /// those as "nothing of ours is stored"). macOS has no `keyring-core`
    /// backend (it goes through the `security` tool), so the variant only
    /// exists elsewhere. The underlying keyring error never carries the
    /// secret.
    #[cfg(not(target_os = "macos"))]
    #[error(transparent)]
    Keyring(#[from] keyring_core::Error),

    /// The macOS `security` tool reported a Keychain failure. Its message is
    /// an OSStatus description and never carries the secret.
    #[cfg(target_os = "macos")]
    #[error(transparent)]
    SecurityTool(#[from] security_cli::SecurityCliError),

    /// The OS keyring is disabled via `_FLOX_DISABLE_KEYRING`. Treated like a
    /// no-backend condition: writes fail so callers fall back to plaintext, and
    /// no real keyring backend is ever initialized. Used by the test suite to
    /// keep integration tests off the developer's global OS keyring.
    #[error("the OS keyring is disabled")]
    Disabled,

    /// An error injected by `MockStore` for testing.
    #[error("{0}")]
    Mock(String),
}

/// Where the active FloxHub credential came from.
///
/// Determined from the underlying primitives (env var, user file, merged
/// config) rather than the already-merged config value, so the same probe can
/// distinguish a system-config token from a user-file token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialSource {
    /// `FLOX_FLOXHUB_TOKEN` is set in the environment.
    Env,
    /// The token came from the system config (`/etc/flox.toml`).
    SystemConfig,
    /// The token is stored in plain text in the user's `flox.toml`.
    UserConfigPlaintext,
    /// The token is stored in the OS keyring.
    Keyring,
    /// No credential is available from any source.
    None,
}

impl CredentialSource {
    /// The shared "stored in plain text at `<path>`" sentence.
    ///
    /// Used both by `flox auth status` (the [CredentialSource::UserConfigPlaintext]
    /// line) and by the plaintext-fallback warning at login, so the wording stays
    /// identical in both places.
    pub(crate) fn plaintext_notice(plaintext_path: &Path) -> String {
        format!(
            "Credential stored in plain text at '{}'.",
            plaintext_path.display()
        )
    }

    /// The user-facing line for `flox auth status` describing where the active
    /// credential is stored, or `None` when there is no line to show.
    ///
    /// `SystemConfig` and `None` produce no line: the former is an
    /// administrator-provided token the user cannot relocate, and the latter
    /// means there is nothing stored.
    pub fn describe_storage(&self, plaintext_path: &Path) -> Option<String> {
        match self {
            CredentialSource::UserConfigPlaintext => Some(Self::plaintext_notice(plaintext_path)),
            CredentialSource::Keyring => {
                Some("Credential stored in your system keyring.".to_string())
            },
            CredentialSource::Env => Some(
                "Credential read from the FLOX_FLOXHUB_TOKEN environment variable.".to_string(),
            ),
            CredentialSource::SystemConfig | CredentialSource::None => None,
        }
    }
}

/// Storage backend for the FloxHub auth token.
#[enum_dispatch]
trait CredentialStore {
    /// Return the stored token, or `None` when this backend has no token.
    fn get(&self) -> Result<Option<String>, CredentialStoreError>;
    /// Store `token`, replacing any previously stored value.
    fn set(&self, token: &str) -> Result<(), CredentialStoreError>;
    /// Remove the stored token. Idempotent: succeeds when nothing is stored.
    fn remove(&self) -> Result<(), CredentialStoreError>;
}

/// The concrete credential backends.
#[enum_dispatch(CredentialStore)]
#[derive(Debug, Clone)]
enum CredentialStoreImpl {
    /// OS-native encrypted credential store (macOS Keychain / Linux Secret
    /// Service), keyed by the FloxHub base URL.
    Keyring(KeyringStore),
    /// `<config_dir>/flox.toml`, with an explicit `0600` on write.
    Plaintext(PlaintextStore),
    /// In-memory backend for tests; supports injected errors.
    Mock(MockStore),
}

/// The storage backend for a persistent credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenStorage {
    /// Stored in the OS keyring.
    Keyring,
    /// Stored in plaintext configuration, explicitly or as a keyring fallback.
    Plaintext,
}

/// A storage migration to report to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialMigration {
    /// The plaintext token was moved into the keyring.
    Migrated,
    /// The keyring write succeeded, but the plaintext copy could not be removed.
    PlaintextRemains,
}

/// Ready-to-use authentication and any migration notice for this invocation.
#[derive(Debug, Clone)]
pub struct AuthResolution {
    pub context: AuthContext,
    pub migration: Option<CredentialMigration>,
}

/// Credential storage and its non-secret cache for one FloxHub instance.
///
/// Startup, login, and logout coordinate credentials and their cached identity
/// here. Callers use the resulting [AuthContext] and report storage notices.
#[derive(Debug, Clone)]
pub struct CredentialStores {
    keyring: CredentialStoreImpl,
    plaintext: CredentialStoreImpl,
    cache: AuthCache,
    /// The user config directory, retained for user-facing messages that name
    /// the plaintext file path.
    config_dir: PathBuf,
}

impl CredentialStores {
    /// Build storage and caching from the FloxHub URL and CLI directories.
    ///
    /// Used at startup, before a [Flox] exists; [Self::from_flox] is the
    /// convenience for the command handlers that already hold one.
    pub fn new(
        floxhub_url: &Url,
        config_dir: impl Into<PathBuf>,
        cache_dir: impl AsRef<Path>,
    ) -> Self {
        let config_dir = config_dir.into();
        Self {
            keyring: CredentialStoreImpl::Keyring(KeyringStore::new(floxhub_url)),
            plaintext: CredentialStoreImpl::Plaintext(PlaintextStore::new(config_dir.clone())),
            cache: AuthCache::new(cache_dir, floxhub_url),
            config_dir,
        }
    }

    /// Build storage and caching for an existing [Flox].
    pub fn from_flox(flox: &Flox) -> Self {
        Self::new(flox.floxhub.base_url(), &flox.config_dir, &flox.cache_dir)
    }

    /// Path to the plaintext `flox.toml`, for user-facing messages about where
    /// a plaintext credential lives.
    pub fn plaintext_path(&self) -> PathBuf {
        self.config_dir.join(FLOX_CONFIG_FILE)
    }

    /// Determine where the active FloxHub credential comes from.
    ///
    /// Read-only, but may access the keyring. Used by logout and auth status.
    ///
    /// Environment and user-file tokens are identified first. Otherwise, a
    /// keyring entry is reported only if the merged token is absent or matches
    /// it. A differing configured token comes from the system configuration,
    /// so messages must not point at an unrelated saved keyring credential.
    pub fn probe_source(&self, config: &Config) -> CredentialSource {
        let env_token = std::env::var(FLOXHUB_TOKEN_ENV_VAR).ok();
        if env_token.is_some_and(|t| !t.is_empty()) {
            return CredentialSource::Env;
        }

        if self.plaintext.get().ok().flatten().is_some() {
            return CredentialSource::UserConfigPlaintext;
        }

        let merged_token = config
            .flox
            .floxhub_token
            .as_deref()
            .filter(|t| !t.is_empty());

        if let Ok(Some(keyring_token)) = self.keyring.get()
            && merged_token.is_none_or(|t| t == keyring_token)
        {
            return CredentialSource::Keyring;
        }

        // The merged config still has a token, but it is not from the
        // environment, the user file, or the keyring — so it came from
        // `/etc/flox.toml`.
        if merged_token.is_some() {
            return CredentialSource::SystemConfig;
        }

        CredentialSource::None
    }

    /// Persist a bearer credential and its resolved identity.
    ///
    /// Cache writes are best effort and happen only after credential storage
    /// succeeds. The returned backend lets the caller explain plaintext storage.
    pub fn persist_login(
        &self,
        context: &AuthContext,
        target: TokenStorageMode,
    ) -> Result<TokenStorage, CredentialStoreError> {
        let token = context
            .token_secret()
            .expect("login completes with a bearer credential");
        let storage = self.persist_login_token(token, target)?;
        self.cache.write(context, storage);
        Ok(storage)
    }

    /// Persist a logged-in token according to `target`.
    ///
    /// `Keyring`: attempt the keyring first (try-then-confirm); on success
    /// store there and remove any lingering plaintext token so it cannot
    /// shadow the keyring entry, and on any keyring failure fall back to the
    /// plaintext file (`0600`). `Plaintext`: write the plaintext file and drop
    /// any existing keyring entry (best effort). The returned [TokenStorage]
    /// tells the caller whether to warn the user.
    fn persist_login_token(
        &self,
        token: &str,
        target: TokenStorageMode,
    ) -> Result<TokenStorage, CredentialStoreError> {
        if target == TokenStorageMode::Keyring && self.keyring.set(token).is_ok() {
            // The keyring already holds the token, so a failure to remove the
            // old plaintext copy must not fail the login. Warn instead: a
            // lingering plaintext token both leaves a secret on disk and shadows
            // the keyring on the next read (user file > keyring).
            if let Err(e) = self.plaintext.remove() {
                tracing::warn!(
                    error = %e,
                    "could not remove the plaintext credential after a keyring write"
                );
                message::warning(indoc! {"
                    Stored your credential in the system keyring.
                    The existing plain-text credential in flox.toml could not be removed.
                    Remove 'floxhub_token' from flox.toml so it does not shadow the keyring."});
            }
            return Ok(TokenStorage::Keyring);
        }

        self.plaintext.set(token)?;
        // An explicit plain-text choice supersedes any keyring entry: drop a
        // lingering keyring token (best effort) so it is not left behind as a
        // stale secret, and is not surfaced on a later read if the plain-text
        // file is removed. (The plain-text file already takes read precedence
        // over the keyring, so this is cleanup, not shadowing.) Scoped to the
        // explicit `Plaintext` target — on a keyring-write fallback there is
        // nothing of ours in the keyring to remove.
        if target == TokenStorageMode::Plaintext
            && let Err(e) = self.keyring.remove()
        {
            tracing::debug!(
                error = %e,
                "could not remove the keyring credential after storing plain text"
            );
        }
        Ok(TokenStorage::Plaintext)
    }

    /// Prepare bearer authentication, including storage migration and caching.
    ///
    /// Call only in token-auth mode and outside prompt hooks. An explicit
    /// environment override, including an empty one, bypasses both secret stores.
    /// Only user-file tokens are eligible for migration; the merged configuration
    /// is never rewritten. Keyring reads are deferred when a usable record exists.
    pub fn resolve(&self, config: &Config) -> AuthResolution {
        let token = config
            .flox
            .floxhub_token
            .as_deref()
            .filter(|token| !token.is_empty());
        let configured = || AuthContext::new_from_token(token);

        if std::env::var(FLOXHUB_TOKEN_ENV_VAR).is_ok() {
            return AuthResolution {
                context: self.cache.configure(configured(), TokenStorage::Plaintext),
                migration: None,
            };
        }

        if config.flox.floxhub_token_storage == TokenStorageMode::Keyring
            && let Ok(Some(token)) = self.plaintext.get()
        {
            let migration = self.migrate_plaintext(&token);
            let storage = if migration.is_some() {
                TokenStorage::Keyring
            } else {
                TokenStorage::Plaintext
            };
            let context = self.cache.configure(configured(), storage);
            if migration.is_some() {
                self.cache.write(&context, storage);
            }
            return AuthResolution { context, migration };
        }

        let context = if token.is_some() {
            self.cache.configure(configured(), TokenStorage::Plaintext)
        } else {
            self.defer_to_keyring()
        };
        AuthResolution {
            context,
            migration: None,
        }
    }

    fn migrate_plaintext(&self, token: &str) -> Option<CredentialMigration> {
        // Confirm the keyring write before removing the only persistent copy.
        self.keyring.set(token).ok()?;
        Some(match self.plaintext.remove() {
            Ok(()) => CredentialMigration::Migrated,
            Err(err) => {
                tracing::warn!(
                    error = %err,
                    "could not remove the plaintext credential after migrating it to the keyring"
                );
                CredentialMigration::PlaintextRemains
            },
        })
    }

    /// A keyring record can answer startup checks without reading the secret.
    /// On a miss, read immediately so the context describes actual credentials.
    fn defer_to_keyring(&self) -> AuthContext {
        let cache = &self.cache;
        let keyring = self.keyring.clone();
        let cache_for_resolver = cache.clone();
        let read_keyring = move || {
            let token = match keyring.get() {
                Ok(token) => token,
                Err(err) => {
                    tracing::debug!(error = %err, "could not read the credential from the keyring");
                    // A failed read says nothing about the stored credential.
                    // Retry next invocation instead of caching a logged-out state.
                    cache_for_resolver.remove();
                    return AuthContext::default();
                },
            };
            let context = cache_for_resolver.configure(
                AuthContext::new_from_token(token.as_deref()),
                TokenStorage::Keyring,
            );
            cache_for_resolver.write(&context, TokenStorage::Keyring);
            context
        };

        cache.defer_or_resolve(read_keyring)
    }

    /// Clear saved authentication and report the source that was active.
    ///
    /// Invalidate the record even when no credential remains or removal fails.
    /// Environment and system-config tokens are not removed; the caller uses
    /// the returned source to explain how to finish logging out.
    pub fn logout(&self, config: &Config) -> Result<CredentialSource, CredentialStoreError> {
        let source = self.probe_source(config);
        self.cache.remove();
        if source != CredentialSource::None {
            self.remove_all()?;
        }
        Ok(source)
    }

    /// Remove the token from both stores, for logout.
    ///
    /// A plaintext token may linger alongside a keyring credential, so both
    /// stores are cleared. Both removals are idempotent.
    ///
    /// Both removals are always attempted: a keyring platform error (e.g. a
    /// locked Secret Service session) must not short-circuit logout and leave
    /// the plaintext secret on disk. A plaintext failure is reported first —
    /// that is the copy sitting in a file.
    fn remove_all(&self) -> Result<(), CredentialStoreError> {
        let keyring_result = self.keyring.remove();
        self.plaintext.remove()?;
        keyring_result
    }
}

/// Helpers shared by this module's tests and the per-backend tests in
/// [keyring], [plaintext], and [mock].
#[cfg(test)]
pub(crate) mod test_helpers {
    use std::path::Path;

    use flox_config::FLOX_CONFIG_FILE;

    /// An opaque token. The store reads/writes arbitrary strings; the probe is
    /// presence-based. Neither needs a JWT-shaped value.
    pub(crate) const TOKEN: &str = "opaque-token-value";

    /// Write a `flox.toml` with the given contents into `dir`.
    pub(crate) fn write_flox_toml(dir: &Path, contents: &str) {
        std::fs::write(dir.join(FLOX_CONFIG_FILE), contents).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use floxhub_client::auth::storage::{AuthContextStorageExt, CachedFacts};
    use pretty_assertions::assert_eq;
    use tempfile::TempDir;

    use super::test_helpers::{TOKEN, write_flox_toml};
    use super::*;

    /// The FloxHub instance the deferred-resolution tests key their record on.
    fn test_account() -> Url {
        Url::parse("https://hub.flox.dev").unwrap()
    }

    impl CredentialStores {
        /// Assemble the pair from arbitrary backends (typically [MockStore]) so
        /// the orchestration methods can be exercised without a real keyring or
        /// FloxHub URL. `config_dir` is left empty because these tests do not
        /// exercise the path-bearing messages.
        fn from_stores(
            keyring: CredentialStoreImpl,
            plaintext: CredentialStoreImpl,
            cache_dir: &Path,
        ) -> Self {
            Self {
                keyring,
                plaintext,
                config_dir: PathBuf::new(),
                cache: AuthCache::new(cache_dir, &test_account()),
            }
        }
    }

    // --- CredentialStores::probe_source: the four Phase 1 input shapes ---
    //
    // Driven through the public `Config::parse()` under `temp_env::with_vars`,
    // mirroring `test_set_by_env` (config/mod.rs:555): `HOME`,
    // `FLOX_CONFIG_DIR` (user dir), and `FLOX_SYSTEM_CONFIG_DIR` (system dir)
    // are set so parsing is hermetic.

    /// Common env scaffolding for a probe test.
    fn probe_vars<'a>(
        home: &'a Path,
        user_dir: &'a Path,
        system_dir: &'a Path,
        floxhub_token: Option<&'a str>,
    ) -> Vec<(&'a str, Option<&'a str>)> {
        vec![
            ("HOME", Some(home.to_str().unwrap())),
            ("FLOX_CONFIG_DIR", Some(user_dir.to_str().unwrap())),
            ("FLOX_SYSTEM_CONFIG_DIR", Some(system_dir.to_str().unwrap())),
            ("FLOX_FLOXHUB_TOKEN", floxhub_token),
        ]
    }

    #[test]
    fn probe_returns_none_when_no_token_anywhere() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let cache_dir = TempDir::new().unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());
                assert_eq!(stores.probe_source(&config), CredentialSource::None);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    #[test]
    fn probe_returns_env_when_env_var_set() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        // A user-file token is also present to prove env wins over it.
        write_flox_toml(user_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(
                home.path(),
                user_dir.path(),
                system_dir.path(),
                Some("env-token"),
            ),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let cache_dir = TempDir::new().unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());
                assert_eq!(stores.probe_source(&config), CredentialSource::Env);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    #[test]
    fn probe_returns_user_config_plaintext_for_user_file_token() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let cache_dir = TempDir::new().unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());
                assert_eq!(
                    stores.probe_source(&config),
                    CredentialSource::UserConfigPlaintext
                );
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// The token comes from the system config only: the merged config has it,
    /// but the user-file probe (`PlaintextStore::get`) does not. Modeled on
    /// `set_by_system_config` (config/mod.rs:581).
    #[test]
    fn probe_returns_system_config_when_token_only_from_system() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                // Sanity: the token reached the merged config from /etc only.
                assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));

                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(plaintext.get().unwrap(), None);
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                let cache_dir = TempDir::new().unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());
                assert_eq!(stores.probe_source(&config), CredentialSource::SystemConfig);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// The keyring is consulted last: with env and both config files empty, a
    /// token in the (mocked) keyring resolves to `Keyring`. Models the
    /// `status reports keyring source` validation item.
    #[test]
    fn probe_returns_keyring_when_only_keyring_has_token() {
        let home = tempfile::tempdir().unwrap();
        let user_dir = tempfile::tempdir().unwrap();
        let system_dir = tempfile::tempdir().unwrap();
        write_flox_toml(user_dir.path(), "");
        write_flox_toml(system_dir.path(), "");

        temp_env::with_vars(
            probe_vars(home.path(), user_dir.path(), system_dir.path(), None),
            || {
                let config = Config::parse().unwrap();
                let plaintext =
                    CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                let keyring = CredentialStoreImpl::Mock(MockStore::new());
                keyring.set(TOKEN).unwrap();
                let cache_dir = TempDir::new().unwrap();
                let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());
                assert_eq!(stores.probe_source(&config), CredentialSource::Keyring);
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// Source probing recognizes a keyring credential even though resolution
    /// leaves it out of the merged configuration.
    #[test]
    fn probe_after_resolver_reports_keyring_not_system_config() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let plaintext = CredentialStoreImpl::Mock(MockStore::new());
        let cache_dir = TempDir::new().unwrap();
        let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());

        let config = config_with_token(None);
        stores.resolve(&config);

        assert_eq!(stores.probe_source(&config), CredentialSource::Keyring);
    }

    /// `/etc/flox.toml` supplies the merged token while the keyring holds a
    /// *different* credential: the probe must report `SystemConfig`, not
    /// `Keyring`, so messaging about a system-supplied token never points at
    /// the user's unrelated saved keyring credential.
    #[test]
    fn probe_reports_system_config_when_keyring_holds_different_token() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext, cache_dir.path());

            // Mirror the merge: the system config supplied the (invalid) token,
            // and the resolver leaves a non-empty merged token untouched.
            let config = config_with_token(Some("invalid-system-token"));
            assert_eq!(stores.resolve(&config).migration, None);

            let source = stores.probe_source(&config);
            assert_eq!(source, CredentialSource::SystemConfig);
        });
    }

    // --- CredentialStores::persist_login_token: the login storage decision ---

    /// Default login: the token goes to the keyring and no plaintext token is
    /// left behind.
    #[test]
    fn login_stores_in_keyring_and_clears_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        // A pre-existing plaintext token must be removed once the keyring write
        // confirms, so it cannot shadow the keyring entry on the next read.
        write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));

        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let cache_dir = TempDir::new().unwrap();
        let stores =
            CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Keyring);
        assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
        assert_eq!(plaintext.get().unwrap(), None);
    }

    /// On any keyring failure, login falls back to plaintext and signals it so
    /// the caller can warn.
    #[test]
    fn login_falls_back_to_plaintext_on_keyring_error() {
        let dir = tempfile::tempdir().unwrap();
        let keyring_mock = MockStore::new();
        keyring_mock.set_error("no backend");
        let keyring = CredentialStoreImpl::Mock(keyring_mock);
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let cache_dir = TempDir::new().unwrap();
        let stores =
            CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// A keyring write succeeds but the plaintext cleanup fails (e.g. the
    /// config file is unreadable). Login must still succeed — the token is
    /// already safely stored in the keyring — rather than turning a best-effort
    /// cleanup step into a hard login failure.
    #[test]
    fn login_succeeds_when_keyring_stored_but_plaintext_cleanup_fails() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext_mock = MockStore::new();
        plaintext_mock.set_error("config file is unreadable");
        let plaintext = CredentialStoreImpl::Mock(plaintext_mock);
        let cache_dir = TempDir::new().unwrap();
        let stores = CredentialStores::from_stores(keyring.clone(), plaintext, cache_dir.path());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Keyring)
            .unwrap();

        assert_eq!(storage, TokenStorage::Keyring);
        assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// A `Plaintext` target forces the plaintext file even when the keyring
    /// write would have succeeded; the keyring is never written.
    #[test]
    fn login_plaintext_target_forces_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let cache_dir = TempDir::new().unwrap();
        let stores =
            CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Plaintext)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// Storing plain text drops any pre-existing keyring entry so it is not left
    /// behind as a stale secret (e.g. to resurface on a later read if the
    /// plain-text file is removed).
    #[test]
    fn login_plaintext_target_removes_stale_keyring_entry() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set("stale-keyring-token").unwrap();
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
        let cache_dir = TempDir::new().unwrap();
        let stores =
            CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

        let storage = stores
            .persist_login_token(TOKEN, TokenStorageMode::Plaintext)
            .unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    // --- CredentialStores::resolve: the upstream read resolver ---

    fn config_with_token(token: Option<&str>) -> Config {
        let mut config = Config::default();
        config.flox.floxhub_token = token.map(str::to_string);
        config
    }

    /// Without a configured token or a record, resolve the keyring credential.
    #[test]
    fn resolve_reads_the_keyring_when_config_and_cache_are_empty() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set(TOKEN).unwrap();
            // Empty plaintext store: nothing to migrate, so the read path runs.
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let cache_dir = TempDir::new().unwrap();
            let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());

            let config = config_with_token(None);
            let outcome = stores.resolve(&config);

            assert_eq!(
                (outcome.context.token_secret(), outcome.migration),
                (Some(TOKEN), None),
            );
            assert_eq!(config.flox.floxhub_token.as_deref(), None);
        });
    }

    #[test]
    fn keyring_read_failure_is_retried_on_the_next_invocation() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            for cached in [false, true] {
                let cache_dir = TempDir::new().unwrap();
                let keyring = MockStore::new();
                keyring.set(TOKEN).unwrap();
                keyring.set_error("keyring is locked");
                let stores = CredentialStores::from_stores(
                    CredentialStoreImpl::Mock(keyring),
                    CredentialStoreImpl::Mock(MockStore::new()),
                    cache_dir.path(),
                );
                let expected = AuthContext::new_from_token(Some(TOKEN));
                if cached {
                    stores.cache.write(&expected, TokenStorage::Keyring);
                }

                let failed = stores.resolve(&config_with_token(None)).context;
                assert_eq!(failed.token_secret(), None);
                assert_eq!(failed.cached_facts(), AuthContext::default().cached_facts());

                let probe = AuthContext::new_from_token(Some("flox_pat_cache-miss-probe"));
                assert_eq!(
                    stores
                        .cache
                        .defer_or_resolve(|| {
                            AuthContext::new_from_token(Some("flox_pat_cache-miss-probe"))
                        })
                        .cached_facts(),
                    probe.cached_facts(),
                    "a failed read must leave a cache miss"
                );

                // Startup facts alone must retry, without asking for the secret.
                let recovered = stores.resolve(&config_with_token(None)).context;
                assert_eq!(recovered.cached_facts(), expected.cached_facts());
                assert_eq!(
                    stores
                        .cache
                        .defer_or_resolve(|| panic!("successful read was not cached"))
                        .cached_facts(),
                    expected.cached_facts()
                );
            }
        });
    }

    #[test]
    fn an_empty_keyring_is_cached_as_logged_out() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let cache_dir = TempDir::new().unwrap();
            let stores = CredentialStores::from_stores(
                CredentialStoreImpl::Mock(MockStore::new()),
                CredentialStoreImpl::Mock(MockStore::new()),
                cache_dir.path(),
            );

            let context = stores.resolve(&config_with_token(None)).context;
            assert_eq!(
                context.cached_facts(),
                AuthContext::default().cached_facts()
            );
            assert_eq!(
                stores
                    .cache
                    .defer_or_resolve(|| panic!("empty keyring was not cached"))
                    .cached_facts(),
                AuthContext::default().cached_facts()
            );
        });
    }

    /// With nothing recorded, the deferred credential still resolves — the
    /// keyring is read straight away and the summary recorded, so only the
    /// first invocation pays.
    #[test]
    fn deferring_without_a_record_reads_the_keyring_and_records_it() {
        let cache_dir = TempDir::new().unwrap();
        let cache = AuthCache::new(cache_dir.path(), &test_account());
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let stores = CredentialStores::from_stores(
            keyring,
            CredentialStoreImpl::Mock(MockStore::new()),
            cache_dir.path(),
        );

        let credential = stores.resolve(&config_with_token(None)).context;

        assert_eq!(credential.token_secret(), Some(TOKEN));
        let recorded =
            cache.defer_or_resolve(|| panic!("the keyring read should have populated the cache"));
        assert!(recorded.cached_facts().logged_in);
        assert_eq!(recorded.cached_facts().expires_at, None);
        assert_eq!(recorded.user_subject(), None);
        assert_eq!(
            recorded.kind(),
            floxhub_client::CredentialKind::OpaqueToken,
            "the mock keyring holds an opaque, prefix-less token"
        );
    }

    /// With a record in hand the keyring is not touched at all: the recorded
    /// answer stands even though the store here holds nothing.
    #[test]
    fn a_record_answers_without_reading_the_keyring() {
        let cache_dir = TempDir::new().unwrap();
        let cache = AuthCache::new(cache_dir.path(), &test_account());
        cache.write(
            &AuthContext::deferred(
                CachedFacts {
                    logged_in: true,
                    kind: floxhub_client::CredentialKind::Auth0,
                    requires_login: true,
                    subject: Some("auth0|deferred".to_string()),
                    ..CachedFacts::default()
                },
                || panic!("recording cached properties must not resolve the credential"),
            ),
            TokenStorage::Keyring,
        );
        let stores = CredentialStores::from_stores(
            CredentialStoreImpl::Mock(MockStore::new()),
            CredentialStoreImpl::Mock(MockStore::new()),
            cache_dir.path(),
        );

        let credential = stores.resolve(&config_with_token(None)).context;

        assert!(!credential.is_unauthenticated());
        assert_eq!(
            credential.user_subject(),
            Some("auth0|deferred".to_string())
        );
    }

    #[test]
    fn a_keyring_read_preserves_matching_plaintext_identity() {
        let cache_dir = TempDir::new().unwrap();
        let cache = AuthCache::new(cache_dir.path(), &test_account());
        let secret = "flox_pat_plaintext-to-keyring";
        let facts = CachedFacts {
            handle: Some("migrated-user".into()),
            subject: Some("account|migrated".into()),
            ..AuthContext::new_from_token(Some(secret)).cached_facts()
        };
        cache.write(
            &AuthContext::deferred(facts.clone(), || {
                panic!("recording must not load the credential")
            }),
            TokenStorage::Plaintext,
        );
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(secret).unwrap();
        let stores = CredentialStores::from_stores(
            keyring,
            CredentialStoreImpl::Mock(MockStore::new()),
            cache_dir.path(),
        );

        let context = stores.resolve(&config_with_token(None)).context;

        assert_eq!(context.cached_facts(), facts);
        let next = cache.defer_or_resolve(|| panic!("the record now describes the keyring"));
        assert_eq!(next.cached_facts(), facts);
    }

    /// A non-empty merged token wins: env > user file > system all flow through this
    /// field, so the keyring is not consulted and the value is untouched.
    #[test]
    fn resolve_leaves_existing_token_untouched() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            // Empty plaintext store: no migration, so only the read path could
            // touch the config — and it must not, because the token is set.
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            let cache_dir = TempDir::new().unwrap();
            let stores = CredentialStores::from_stores(keyring, plaintext, cache_dir.path());

            let config = config_with_token(Some("config-token"));
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, None);
            assert_eq!(config.flox.floxhub_token.as_deref(), Some("config-token"));
        });
    }

    // --- CredentialStores::resolve: opportunistic plaintext → keyring migration ---

    /// A user-file plaintext token is moved into the keyring and removed from
    /// the file once the keyring write confirms.
    #[test]
    fn resolve_migrates_plaintext_token_to_keyring() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());

            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            // Mirror the merge: the user-file token is already in the config.
            let config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, Some(CredentialMigration::Migrated));
            assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
            assert_eq!(plaintext.get().unwrap(), None);
            // Migration is store-to-store only: the config field is left as the
            // merge produced it.
            assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));
        });
    }

    /// When the standing storage preference is plain text, a user-file token is
    /// not migrated into the keyring: the keyring is never written and the
    /// plain-text token stays on disk.
    #[test]
    fn resolve_skips_migration_when_storage_is_plaintext() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            let mut config = config_with_token(Some(TOKEN));
            config.flox.floxhub_token_storage = TokenStorageMode::Plaintext;
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, None);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// `FLOX_FLOXHUB_TOKEN` set → the plaintext token is not migrated (the env
    /// token is transient and must not be persisted to the keyring).
    #[test]
    fn resolve_does_not_migrate_when_env_token_set() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, Some("env-token"), || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring = CredentialStoreImpl::Mock(MockStore::new());

            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            let config = config_with_token(Some("env-token"));
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, None);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// An explicit *empty* `FLOX_FLOXHUB_TOKEN` export masks saved credentials
    /// for one invocation: the resolver must neither migrate the plaintext
    /// token nor populate the config from the keyring, so the invocation stays
    /// logged out.
    #[test]
    fn resolve_is_inert_when_env_token_is_empty() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, Some(""), || {
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            keyring.set("keyring-token").unwrap();
            let plaintext = CredentialStoreImpl::Mock(MockStore::new());
            plaintext.set(TOKEN).unwrap();
            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            // Mirror the merge: the empty env override yields an empty merged
            // token.
            let config = config_with_token(Some(""));
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, None);
            // No migration: both stores are exactly as they were.
            assert_eq!(keyring.get().unwrap(), Some("keyring-token".to_string()));
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
            // No populate: the masked (empty) token is left in place.
            assert_eq!(config.flox.floxhub_token.as_deref(), Some(""));
        });
    }

    /// Keyring write fails → the plaintext file is left untouched (no data
    /// loss, no migration).
    #[test]
    fn resolve_leaves_plaintext_untouched_when_keyring_write_fails() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let dir = tempfile::tempdir().unwrap();
            write_flox_toml(dir.path(), &format!("floxhub_token = \"{TOKEN}\"\n"));
            let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));
            let keyring_mock = MockStore::new();
            // The injected error lands on the migration's `set` call — the
            // migration branch never calls `keyring.get()` first.
            keyring_mock.set_error("no backend");
            let keyring = CredentialStoreImpl::Mock(keyring_mock);
            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            let config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve(&config);

            assert_eq!(outcome.migration, None);
            assert_eq!(keyring.get().unwrap(), None);
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    /// Keyring write succeeds but the plaintext removal fails (e.g. an unwritable
    /// `flox.toml`). The token is now in the keyring, but the plaintext copy
    /// lingers, so the caller must report the incomplete migration.
    #[test]
    fn resolve_reports_plaintext_remains_when_remove_fails_after_keyring_write() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let plaintext_mock = MockStore::new();
            plaintext_mock.set(TOKEN).unwrap();
            plaintext_mock.set_remove_error("flox.toml is not writable");
            let plaintext = CredentialStoreImpl::Mock(plaintext_mock);
            let keyring = CredentialStoreImpl::Mock(MockStore::new());
            let cache_dir = TempDir::new().unwrap();
            let stores =
                CredentialStores::from_stores(keyring.clone(), plaintext.clone(), cache_dir.path());

            let config = config_with_token(Some(TOKEN));
            let outcome = stores.resolve(&config);

            assert_eq!(
                outcome.migration,
                Some(CredentialMigration::PlaintextRemains)
            );
            // The keyring received the token; the plaintext copy still lingers.
            assert_eq!(keyring.get().unwrap(), Some(TOKEN.to_string()));
            assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
        });
    }

    // --- CredentialStores::remove_all: logout removal ---

    #[test]
    fn logout_drops_stale_identity_when_credentials_are_already_gone() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let cache_dir = TempDir::new().unwrap();
            let stores = CredentialStores::from_stores(
                CredentialStoreImpl::Mock(MockStore::new()),
                CredentialStoreImpl::Mock(MockStore::new()),
                cache_dir.path(),
            );
            stores.cache.write(
                &AuthContext::new_from_token(Some(TOKEN)),
                TokenStorage::Keyring,
            );

            let source = stores.logout(&config_with_token(None)).unwrap();

            assert_eq!(
                (
                    source,
                    stores
                        .cache
                        .defer_or_resolve(AuthContext::default)
                        .cached_facts()
                ),
                (
                    CredentialSource::None,
                    AuthContext::default().cached_facts()
                ),
            );
        });
    }

    #[test]
    fn logout_invalidates_identity_even_when_a_store_cannot_be_cleared() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            for failing_store in [TokenStorage::Keyring, TokenStorage::Plaintext] {
                let cache_dir = TempDir::new().unwrap();
                let keyring = MockStore::new();
                let plaintext = MockStore::new();
                keyring.set(TOKEN).unwrap();
                plaintext.set(TOKEN).unwrap();
                match failing_store {
                    TokenStorage::Keyring => keyring.set_remove_error("keyring is locked"),
                    TokenStorage::Plaintext => plaintext.set_remove_error("config is read-only"),
                }
                let stores = CredentialStores::from_stores(
                    CredentialStoreImpl::Mock(keyring.clone()),
                    CredentialStoreImpl::Mock(plaintext.clone()),
                    cache_dir.path(),
                );
                stores.cache.write(
                    &AuthContext::new_from_token(Some(TOKEN)),
                    TokenStorage::Keyring,
                );

                let result = stores.logout(&config_with_token(Some(TOKEN)));

                assert_eq!(
                    (
                        result.is_err(),
                        keyring.get().unwrap(),
                        plaintext.get().unwrap(),
                        stores
                            .cache
                            .defer_or_resolve(AuthContext::default)
                            .cached_facts(),
                    ),
                    (
                        true,
                        (failing_store == TokenStorage::Keyring).then(|| TOKEN.to_string()),
                        (failing_store == TokenStorage::Plaintext).then(|| TOKEN.to_string()),
                        AuthContext::default().cached_facts(),
                    ),
                );
            }
        });
    }

    #[test]
    fn failed_login_preserves_the_previous_identity_record() {
        let cache_dir = TempDir::new().unwrap();
        let keyring = MockStore::new();
        let plaintext = MockStore::new();
        keyring.set_error("keyring is locked");
        plaintext.set_error("config is read-only");
        let stores = CredentialStores::from_stores(
            CredentialStoreImpl::Mock(keyring),
            CredentialStoreImpl::Mock(plaintext),
            cache_dir.path(),
        );
        let previous = AuthContext::new_from_token(Some(TOKEN));
        stores.cache.write(&previous, TokenStorage::Keyring);

        let result = stores.persist_login(
            &AuthContext::new_from_token(Some("flox_pat_cannot-save")),
            TokenStorageMode::Keyring,
        );

        assert_eq!(
            (
                result.is_err(),
                stores
                    .cache
                    .defer_or_resolve(AuthContext::default)
                    .cached_facts()
            ),
            (true, previous.cached_facts()),
        );
    }

    #[test]
    fn migration_returns_cached_identity_and_records_the_keyring_backend() {
        temp_env::with_var(FLOXHUB_TOKEN_ENV_VAR, None::<&str>, || {
            let cache_dir = TempDir::new().unwrap();
            let stores = CredentialStores::from_stores(
                CredentialStoreImpl::Mock(MockStore::new()),
                CredentialStoreImpl::Mock(MockStore::new()),
                cache_dir.path(),
            );
            let secret = "flox_pat_migration-with-identity";
            let context = AuthContext::new_from_token(Some(secret));
            let facts = CachedFacts {
                handle: Some("migration-user".into()),
                subject: Some("account|migration".into()),
                ..context.cached_facts()
            };
            context.seed_from(&facts);
            stores
                .persist_login(&context, TokenStorageMode::Plaintext)
                .unwrap();

            let resolved = stores.resolve(&config_with_token(Some(secret)));

            assert_eq!(
                (
                    resolved.context.cached_facts(),
                    resolved.migration,
                    stores.keyring.get().unwrap(),
                    stores.plaintext.get().unwrap(),
                    stores
                        .cache
                        .defer_or_resolve(|| panic!("migration must record keyring state"))
                        .cached_facts(),
                ),
                (
                    facts.clone(),
                    Some(CredentialMigration::Migrated),
                    Some(secret.to_string()),
                    None,
                    facts,
                ),
            );
        });
    }

    /// A keyring platform failure (e.g. a locked Secret Service session) must
    /// not short-circuit logout: the plaintext token is still removed, and the
    /// keyring error is still surfaced to the caller.
    #[test]
    fn remove_all_clears_plaintext_even_when_keyring_remove_fails() {
        let keyring_mock = MockStore::new();
        keyring_mock.set_error("keyring is locked");
        let keyring = CredentialStoreImpl::Mock(keyring_mock);
        let plaintext = CredentialStoreImpl::Mock(MockStore::new());
        plaintext.set(TOKEN).unwrap();
        let cache_dir = TempDir::new().unwrap();
        let stores = CredentialStores::from_stores(keyring, plaintext.clone(), cache_dir.path());

        let result = stores.remove_all();

        assert!(result.is_err());
        assert_eq!(plaintext.get().unwrap(), None);
    }
}
