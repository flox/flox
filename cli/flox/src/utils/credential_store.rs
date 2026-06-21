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
//! Phase 1 ships the plaintext and mock backends only. The keyring backend
//! (and the `CredentialSource::Keyring` probe branch) is added in Phase 2.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use enum_dispatch::enum_dispatch;
use flox_rust_sdk::models::floxmeta::FLOXHUB_TOKEN_ENV_VAR;
use thiserror::Error;

use crate::commands::general::update_config;
use flox_config::{Config, FLOX_CONFIG_FILE};

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
    /// The token is stored in the OS keyring. Constructed in Phase 2 once the
    /// keyring backend exists; kept here so the `status` match is forward-ready.
    #[allow(dead_code)]
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
///
/// `Keyring(KeyringStore)` is added in Phase 2.
#[enum_dispatch(CredentialStore)]
#[derive(Debug, Clone)]
pub enum CredentialStoreImpl {
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
/// Precedence (Phase 1): `FLOX_FLOXHUB_TOKEN` env > user-file plaintext >
/// system config > none. The `Keyring` branch is added in Phase 2.
pub fn probe_credential_source(config: &Config, store: &CredentialStoreImpl) -> CredentialSource {
    let env_token = std::env::var(FLOXHUB_TOKEN_ENV_VAR).ok();
    if env_token.is_some_and(|t| !t.is_empty()) {
        return CredentialSource::Env;
    }

    if store.get().ok().flatten().is_some() {
        return CredentialSource::UserConfigPlaintext;
    }

    // The merged config still has a token, but it is neither from the
    // environment nor the user file — so it came from `/etc/flox.toml`.
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
                let store = CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(
                    probe_credential_source(&config, &store),
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
                let store = CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(
                    probe_credential_source(&config, &store),
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
                let store = CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(
                    probe_credential_source(&config, &store),
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

                let store = CredentialStoreImpl::Plaintext(PlaintextStore::new(user_dir.path()));
                assert_eq!(store.get().unwrap(), None);
                assert_eq!(
                    probe_credential_source(&config, &store),
                    CredentialSource::SystemConfig
                );
                unsafe { env::remove_var("FLOX_CONFIG_DIR") };
            },
        );
    }
}
