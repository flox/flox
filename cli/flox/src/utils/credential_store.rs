//! Storage backend abstraction for the FloxHub auth token.
//!
//! The token's runtime representation (`Option<String>` in [Config] → parsed
//! `FloxhubToken` → `AuthContext`) is unchanged; this module owns only the
//! *source* of the string and the *destination* of writes.
//!
//! The abstraction follows the `enum_dispatch` + `Mock`-arm pattern used by
//! `InstallableLockerImpl`
//! (`cli/flox-rust-sdk/src/providers/flake_installable_locker.rs`), and the
//! typed-error convention used by `AuthError`
//! (`cli/flox-rust-sdk/src/providers/nix_auth.rs`): any "no backend" or
//! credential-redaction concern lives in the error type rather than at call
//! sites.
//!
//! Phase 2 adds the [KeyringStore] backend (OS-native encrypted credential
//! store) and the [CredentialSource::Keyring] probe branch.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Once};

use enum_dispatch::enum_dispatch;
use flox_rust_sdk::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use thiserror::Error;
use url::Url;

use crate::commands::general::update_config;
use crate::config::{Config, FLOX_CONFIG_FILE};

/// `service` value for the FloxHub credential in the OS keyring. The token is
/// keyed by this constant plus the FloxHub base URL as the `account`, mirroring
/// `gh`'s per-host keying so distinct FloxHub instances stay separate.
const KEYRING_SERVICE: &str = "dev.flox.flox";

/// Errors returned by a [CredentialStore].
///
/// Per the project conventions, credential redaction and backend-availability
/// classification belong here rather than at call sites. The underlying writes
/// (`update_config`) never interpolate the token into their messages, so no
/// variant carries the secret.
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

    /// No usable OS keyring backend is available (no default store, platform
    /// failure, or the store could not be accessed). Callers treat this as the
    /// signal to fall back to plaintext storage. The underlying keyring error
    /// never carries the secret.
    #[error("no OS keyring backend is available")]
    NoBackend(#[source] keyring_core::Error),

    /// An unclassified OS keyring failure that is neither "no entry" nor a
    /// known no-backend condition.
    #[error("the OS keyring operation failed")]
    Keyring(#[source] keyring_core::Error),

    /// An error injected by [MockStore] for testing.
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

/// Storage backend for the FloxHub auth token.
#[enum_dispatch]
pub trait CredentialStore {
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
pub enum CredentialStoreImpl {
    /// OS-native encrypted credential store (macOS Keychain / Linux Secret
    /// Service), keyed by the FloxHub base URL.
    Keyring(KeyringStore),
    /// `<config_dir>/flox.toml`, with an explicit `0600` on write.
    Plaintext(PlaintextStore),
    /// In-memory backend for tests; supports injected errors.
    Mock(MockStore),
}

/// Plain-text token storage in `<config_dir>/flox.toml`.
#[derive(Debug, Clone)]
pub struct PlaintextStore {
    config_dir: PathBuf,
}

impl PlaintextStore {
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    /// Path to the `flox.toml` this store reads and writes.
    fn config_file(&self) -> PathBuf {
        self.config_dir.join(FLOX_CONFIG_FILE)
    }
}

/// Read the `floxhub_token` value straight from `flox.toml`.
///
/// This is the *user-file* provenance probe, distinct from the merged
/// [Config] value: it never sees a token contributed by `/etc/flox.toml` or by
/// `FLOX_FLOXHUB_TOKEN`. A missing file, a missing key, or an empty string all
/// resolve to `None`.
fn read_token_from_file(path: &Path) -> Result<Option<String>, CredentialStoreError> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CredentialStoreError::ReadPlaintext(e)),
    };

    let document = contents
        .parse::<toml_edit::DocumentMut>()
        .map_err(CredentialStoreError::ParsePlaintext)?;

    let token = document
        .get("floxhub_token")
        .and_then(|item| item.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    Ok(token)
}

/// Set owner-only (`0600`) permissions on the credential file.
///
/// `write_atomically` already yields `0600` via `tempfile`, but Q7 calls for
/// setting the bits explicitly (and repairing them) on the token-bearing write.
/// Factored out so it can be tested as the *last* writer — proving the explicit
/// chmod runs, which a post-`set()` mode assertion cannot, since the atomic
/// rename would produce `0600` regardless.
#[cfg(unix)]
fn set_token_file_permissions(path: &Path) -> Result<(), CredentialStoreError> {
    use std::os::unix::fs::PermissionsExt;

    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(CredentialStoreError::SetPermissions)
}

#[cfg(not(unix))]
fn set_token_file_permissions(_path: &Path) -> Result<(), CredentialStoreError> {
    Ok(())
}

impl CredentialStore for PlaintextStore {
    fn get(&self) -> Result<Option<String>, CredentialStoreError> {
        read_token_from_file(&self.config_file())
    }

    fn set(&self, token: &str) -> Result<(), CredentialStoreError> {
        update_config(&self.config_dir, "floxhub_token", Some(token))
            .map_err(CredentialStoreError::Plaintext)?;
        set_token_file_permissions(&self.config_file())
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        // `update_config(.., None)` errors when the key is absent. Probe the
        // file first so removal is idempotent — logout must not regress when
        // the token came from the environment or system config rather than the
        // user file.
        if read_token_from_file(&self.config_file())?.is_none() {
            return Ok(());
        }

        update_config::<String>(&self.config_dir, "floxhub_token", None)
            .map_err(CredentialStoreError::Plaintext)
    }
}

/// Register the platform-native keyring as `keyring_core`'s default store.
///
/// Mirrors the `keyring` v4 `v1` module: try the per-target backend once, and
/// swallow construction errors — when no backend registers, [keyring_core::Entry::new]
/// returns [keyring_core::Error::NoDefaultStore], which [KeyringStore] maps to
/// [CredentialStoreError::NoBackend] so the caller falls back to plaintext.
fn register_default_store() {
    static SET_CREDENTIAL_STORE: Once = Once::new();
    SET_CREDENTIAL_STORE.call_once(|| {
        #[cfg(target_os = "macos")]
        {
            if let Ok(store) = apple_native_keyring_store::keychain::Store::new() {
                keyring_core::set_default_store(store);
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(store) = zbus_secret_service_keyring_store::Store::new() {
                keyring_core::set_default_store(store);
            }
        }
    });
}

/// Map a [keyring_core::Error] from a `get`/`set`/`remove` into our typed error.
///
/// [keyring_core::Error::NoEntry] is handled by callers (it is not a failure);
/// the no-backend conditions become [CredentialStoreError::NoBackend] so the
/// caller can branch on the type rather than a string.
fn classify_keyring_error(error: keyring_core::Error) -> CredentialStoreError {
    match error {
        keyring_core::Error::NoDefaultStore
        | keyring_core::Error::PlatformFailure(_)
        | keyring_core::Error::NoStorageAccess(_) => CredentialStoreError::NoBackend(error),
        other => CredentialStoreError::Keyring(other),
    }
}

/// OS-native encrypted credential storage (macOS Keychain / Linux Secret
/// Service) via the `keyring` v4 crates.
///
/// The entry is keyed by [KEYRING_SERVICE] plus the FloxHub base URL as the
/// account, so distinct FloxHub instances do not collide.
#[derive(Debug, Clone)]
pub struct KeyringStore {
    account: String,
}

impl KeyringStore {
    pub fn new(floxhub_url: &Url) -> Self {
        Self {
            account: floxhub_url.as_str().to_string(),
        }
    }

    fn entry(&self) -> Result<keyring_core::Entry, CredentialStoreError> {
        register_default_store();
        keyring_core::Entry::new(KEYRING_SERVICE, &self.account).map_err(classify_keyring_error)
    }
}

impl CredentialStore for KeyringStore {
    fn get(&self) -> Result<Option<String>, CredentialStoreError> {
        match self.entry()?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(classify_keyring_error(e)),
        }
    }

    fn set(&self, token: &str) -> Result<(), CredentialStoreError> {
        // Try-then-confirm: attempt the write directly. Any failure (including
        // a missing backend) surfaces as an error so the caller falls back to
        // plaintext, rather than probing availability up front.
        self.entry()?
            .set_password(token)
            .map_err(classify_keyring_error)
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        // Best-effort across machines with no keyring: when no backend is
        // available there is nothing of ours stored there, so logout still
        // succeeds. A backend that *is* present but rejects the delete (locked,
        // platform error) is surfaced so logout does not falsely claim success.
        let entry = match self.entry() {
            Ok(entry) => entry,
            Err(CredentialStoreError::NoBackend(_)) => return Ok(()),
            Err(e) => return Err(e),
        };
        match entry.delete_credential() {
            // Idempotent: a missing entry is not a failure.
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(classify_keyring_error(e)),
        }
    }
}

/// In-memory credential store for tests, with optional error injection.
#[derive(Debug, Clone, Default)]
pub struct MockStore {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Debug, Default)]
struct MockState {
    token: Option<String>,
    error: Option<String>,
}

impl MockStore {
    // Test-only constructor; exercised by the orchestration tests in Phase 2/3
    // and this module's tests. `cli/flox` is a binary crate, so `pub` does not
    // exempt it from dead-code analysis (mirrors `set_lock_results` in the SDK).
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Inject an error returned by the next `get`/`set`/`remove` call.
    #[allow(dead_code)]
    pub fn set_error(&self, message: impl Into<String>) {
        self.inner.lock().unwrap().error = Some(message.into());
    }

    fn take_error(&self) -> Option<CredentialStoreError> {
        self.inner
            .lock()
            .unwrap()
            .error
            .take()
            .map(CredentialStoreError::Mock)
    }
}

impl CredentialStore for MockStore {
    fn get(&self) -> Result<Option<String>, CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        Ok(self.inner.lock().unwrap().token.clone())
    }

    fn set(&self, token: &str) -> Result<(), CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        self.inner.lock().unwrap().token = Some(token.to_string());
        Ok(())
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        if let Some(e) = self.take_error() {
            return Err(e);
        }
        self.inner.lock().unwrap().token = None;
        Ok(())
    }
}

/// Determine where the active FloxHub credential comes from.
///
/// Pure: no migration or other side effects. Shared by the startup resolver
/// (Phase 2/3) and `flox auth status`.
///
/// Precedence: `FLOX_FLOXHUB_TOKEN` env > user-file plaintext > keyring >
/// system config > none.
///
/// The keyring is probed before the system-config inference because the
/// resolver (`resolve_credential_into`) has, by the time `status` runs,
/// populated `config.flox.floxhub_token` from the keyring when the merged config
/// was empty. The `SystemConfig` inference rests on that field being `Some` for
/// reasons *other* than env/user-file, so it must come last — otherwise a
/// keyring-sourced token would misreport as `SystemConfig`. The only case this
/// reorders is "both `/etc` and the keyring hold a token", which reports
/// `Keyring` though `/etc` shadows it under the read precedence — a cosmetic
/// `status`-only difference.
pub fn probe_credential_source(
    config: &Config,
    plaintext: &CredentialStoreImpl,
    keyring: &CredentialStoreImpl,
) -> CredentialSource {
    let env_token = std::env::var(FLOXHUB_TOKEN_ENV_VAR).ok();
    if env_token.is_some_and(|t| !t.is_empty()) {
        return CredentialSource::Env;
    }

    if plaintext.get().ok().flatten().is_some() {
        return CredentialSource::UserConfigPlaintext;
    }

    if keyring.get().ok().flatten().is_some() {
        return CredentialSource::Keyring;
    }

    // The merged config still has a token, but it is not from the environment,
    // the user file, or the keyring — so it came from `/etc/flox.toml`.
    if config
        .flox
        .floxhub_token
        .as_deref()
        .is_some_and(|t| !t.is_empty())
    {
        return CredentialSource::SystemConfig;
    }

    CredentialSource::None
}

/// Where a freshly-logged-in token was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenStorage {
    /// Stored in the OS keyring.
    Keyring,
    /// Stored in the plaintext `flox.toml` file (the fallback).
    Plaintext,
}

/// Persist a logged-in token to the most secure available store.
///
/// Unless `insecure_storage` forces plaintext, attempt the keyring first
/// (try-then-confirm): on success store there and remove any lingering
/// plaintext token so it cannot shadow the keyring entry. On any keyring
/// failure — or when plaintext is forced — write the plaintext file (`0600`).
/// The returned [TokenStorage] tells the caller whether to warn the user.
pub fn persist_login_token(
    token: &str,
    insecure_storage: bool,
    keyring: &CredentialStoreImpl,
    plaintext: &CredentialStoreImpl,
) -> Result<TokenStorage, CredentialStoreError> {
    if !insecure_storage && keyring.set(token).is_ok() {
        plaintext.remove()?;
        return Ok(TokenStorage::Keyring);
    }

    plaintext.set(token)?;
    Ok(TokenStorage::Plaintext)
}

/// Populate `config.flox.floxhub_token` from the keyring when the merged config
/// supplied no token.
///
/// This is the single upstream resolution step (Q8, Option C): both the loud
/// `resolve_floxhub_token` and the silent `init_floxhub_client` read the same
/// field, so populating it once here lets both see the keyring value with no
/// change to their internals.
///
/// Precedence is preserved by only consulting the keyring when the merged value
/// (env > system > plaintext file) is empty. No keyring I/O happens in the
/// prompt/hook flow.
pub fn resolve_credential_into(config: &mut Config, keyring: &CredentialStoreImpl, is_hook: bool) {
    if is_hook {
        return;
    }

    if config
        .flox
        .floxhub_token
        .as_deref()
        .is_some_and(|t| !t.is_empty())
    {
        return;
    }

    if let Ok(Some(token)) = keyring.get() {
        config.flox.floxhub_token = Some(token);
    }
}

#[cfg(test)]
mod tests {
    use std::env;

    use pretty_assertions::assert_eq;

    use super::*;

    /// An opaque token. The store reads/writes arbitrary strings; the probe is
    /// presence-based. Neither needs a JWT-shaped value.
    const TOKEN: &str = "opaque-token-value";

    fn write_flox_toml(dir: &Path, contents: &str) {
        std::fs::write(dir.join(FLOX_CONFIG_FILE), contents).unwrap();
    }

    #[cfg(unix)]
    fn mode_of(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn plaintext_set_get_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaintextStore::new(dir.path());

        assert_eq!(store.get().unwrap(), None);

        store.set(TOKEN).unwrap();
        assert_eq!(store.get().unwrap(), Some(TOKEN.to_string()));

        store.remove().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn plaintext_remove_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaintextStore::new(dir.path());

        // No file, no key — must still succeed.
        store.remove().unwrap();

        write_flox_toml(dir.path(), "disable_metrics = true\n");
        // File exists but key absent — must still succeed and leave the file.
        store.remove().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn plaintext_get_ignores_empty_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaintextStore::new(dir.path());

        write_flox_toml(dir.path(), "floxhub_token = \"\"\n");
        assert_eq!(store.get().unwrap(), None);
    }

    /// The discriminating permission test: the explicit chmod is the *last*
    /// writer, so a pre-existing broad mode is repaired only if the chmod
    /// actually runs. The atomic rename in `set()` cannot produce this result
    /// because it replaces the inode with a fresh `0600` temp file regardless.
    #[cfg(unix)]
    #[test]
    fn set_token_file_permissions_repairs_broad_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FLOX_CONFIG_FILE);
        std::fs::write(&path, "floxhub_token = \"x\"\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(mode_of(&path), 0o644);

        set_token_file_permissions(&path).unwrap();
        assert_eq!(mode_of(&path), 0o600);
    }

    /// End-to-end post-condition guard: after `set()`, the file is `0600`.
    #[cfg(unix)]
    #[test]
    fn plaintext_set_produces_owner_only_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = PlaintextStore::new(dir.path());

        store.set(TOKEN).unwrap();
        assert_eq!(mode_of(&dir.path().join(FLOX_CONFIG_FILE)), 0o600);
    }

    #[test]
    fn mock_set_get_remove_round_trip() {
        let store = MockStore::new();
        assert_eq!(store.get().unwrap(), None);

        store.set(TOKEN).unwrap();
        assert_eq!(store.get().unwrap(), Some(TOKEN.to_string()));

        store.remove().unwrap();
        assert_eq!(store.get().unwrap(), None);
    }

    #[test]
    fn mock_injects_error() {
        let store = MockStore::new();
        store.set_error("boom");

        let result = store.get();
        assert_eq!(
            result.unwrap_err().to_string(),
            CredentialStoreError::Mock("boom".to_string()).to_string()
        );

        // The injected error is consumed; subsequent calls succeed.
        assert_eq!(store.get().unwrap(), None);
    }

    // --- probe_credential_source: the four Phase 1 input shapes ---
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
                assert_eq!(
                    probe_credential_source(&config, &plaintext, &keyring),
                    CredentialSource::None
                );
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
                assert_eq!(
                    probe_credential_source(&config, &plaintext, &keyring),
                    CredentialSource::Env
                );
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
                assert_eq!(
                    probe_credential_source(&config, &plaintext, &keyring),
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
                assert_eq!(
                    probe_credential_source(&config, &plaintext, &keyring),
                    CredentialSource::SystemConfig
                );
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
                assert_eq!(
                    probe_credential_source(&config, &plaintext, &keyring),
                    CredentialSource::Keyring
                );
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }

    /// Reproduce the real `status` call order: the resolver populates
    /// `config.flox.floxhub_token` from the keyring, then `probe` runs on that
    /// mutated config. The keyring branch must win over the `SystemConfig`
    /// inference — otherwise a keyring-sourced token misreports as system
    /// config. (`Config::parse()` is unnecessary here: the resolver works on
    /// the merged field directly, which is what the bug hinges on.)
    #[test]
    fn probe_after_resolver_reports_keyring_not_system_config() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();
        let plaintext = CredentialStoreImpl::Mock(MockStore::new());

        let mut config = config_with_token(None);
        resolve_credential_into(&mut config, &keyring, false);

        assert_eq!(
            probe_credential_source(&config, &plaintext, &keyring),
            CredentialSource::Keyring
        );
    }

    // --- persist_login_token: the login storage decision ---

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

        let storage = persist_login_token(TOKEN, false, &keyring, &plaintext).unwrap();

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

        let storage = persist_login_token(TOKEN, false, &keyring, &plaintext).unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    /// `--insecure-storage` forces plaintext even when the keyring write would
    /// have succeeded; the keyring is never touched.
    #[test]
    fn login_insecure_storage_forces_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        let plaintext = CredentialStoreImpl::Plaintext(PlaintextStore::new(dir.path()));

        let storage = persist_login_token(TOKEN, true, &keyring, &plaintext).unwrap();

        assert_eq!(storage, TokenStorage::Plaintext);
        assert_eq!(keyring.get().unwrap(), None);
        assert_eq!(plaintext.get().unwrap(), Some(TOKEN.to_string()));
    }

    // --- resolve_credential_into: the upstream read resolver ---

    fn config_with_token(token: Option<&str>) -> Config {
        let mut config = Config::default();
        config.flox.floxhub_token = token.map(str::to_string);
        config
    }

    /// When the merged config supplied no token, the keyring value populates it.
    #[test]
    fn resolve_populates_token_from_keyring_when_config_empty() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();

        let mut config = config_with_token(None);
        resolve_credential_into(&mut config, &keyring, false);

        assert_eq!(config.flox.floxhub_token.as_deref(), Some(TOKEN));
    }

    /// A non-empty merged token wins: env > system > file all flow through this
    /// field, so the keyring is not consulted and the value is untouched.
    #[test]
    fn resolve_leaves_existing_token_untouched() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set("keyring-token").unwrap();

        let mut config = config_with_token(Some("config-token"));
        resolve_credential_into(&mut config, &keyring, false);

        assert_eq!(config.flox.floxhub_token.as_deref(), Some("config-token"));
    }

    /// The prompt/hook flow performs no keyring I/O: an empty config stays
    /// empty even when the keyring holds a token.
    #[test]
    fn resolve_skips_keyring_in_hook_flow() {
        let keyring = CredentialStoreImpl::Mock(MockStore::new());
        keyring.set(TOKEN).unwrap();

        let mut config = config_with_token(None);
        resolve_credential_into(&mut config, &keyring, true);

        assert_eq!(config.flox.floxhub_token, None);
    }
}
