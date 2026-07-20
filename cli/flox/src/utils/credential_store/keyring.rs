//! The OS-native encrypted credential backend (macOS Keychain / Linux Secret
//! Service), via the `keyring` v4 crates.

use std::sync::Once;

use url::Url;

use super::{CredentialStore, CredentialStoreError};

/// `service` value for the FloxHub credential in the OS keyring. The token is
/// keyed by this constant plus the FloxHub base URL as the `account`, mirroring
/// `gh`'s per-host keying so distinct FloxHub instances stay separate.
const KEYRING_SERVICE: &str = "dev.flox.flox";

/// When this environment variable is set to any non-empty value, [KeyringStore]
/// behaves as a no-backend keyring (no OS keyring is ever initialized).
///
/// The OS keyring is global — keyed by the FloxHub URL, not isolated by
/// `FLOX_CONFIG_DIR` — so without this gate integration (bats) tests on a
/// keyring-capable machine would read and clobber the developer's real
/// FloxHub credential. The test suite sets this var so every test run is
/// equivalent to a keyringless box.
const DISABLE_KEYRING_ENV_VAR: &str = "_FLOX_DISABLE_KEYRING";

/// Whether the OS keyring is disabled via [DISABLE_KEYRING_ENV_VAR].
///
/// Any non-empty value counts as "set". Checked before any keyring backend is
/// initialized, so a disabled keyring never triggers an OS unlock prompt.
fn keyring_disabled() -> bool {
    std::env::var(DISABLE_KEYRING_ENV_VAR).is_ok_and(|v| !v.is_empty())
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
        // Disabled keyring: behave as a no-backend box. Return `Ok(None)` (not
        // an error) so this path is deterministic on a developer's keyring-
        // capable machine. Checked before `entry()`, so no backend is
        // initialized and no OS unlock prompt is triggered.
        if keyring_disabled() {
            return Ok(None);
        }
        match self.entry()?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(classify_keyring_error(e)),
        }
    }

    fn set(&self, token: &str) -> Result<(), CredentialStoreError> {
        // Disabled keyring: fail so callers fall back to plaintext. This MUST
        // be an error, not a silent `Ok` — migration is
        // `keyring.set(..).is_ok() && plaintext.remove()`, so a no-op `Ok`
        // would delete the plaintext token while storing nothing.
        if keyring_disabled() {
            return Err(CredentialStoreError::Disabled);
        }
        // Try-then-confirm: attempt the write directly. Any failure (including
        // a missing backend) surfaces as an error so the caller falls back to
        // plaintext, rather than probing availability up front.
        self.entry()?
            .set_password(token)
            .map_err(classify_keyring_error)
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        // Disabled keyring: nothing of ours is stored, so removal is a no-op
        // success — logout must still succeed.
        if keyring_disabled() {
            return Ok(());
        }
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

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::utils::credential_store::test_helpers::TOKEN;

    /// With `_FLOX_DISABLE_KEYRING` set, the *real* `KeyringStore` behaves as a
    /// no-backend box without touching any OS keyring: `get` yields `None`,
    /// `set` is an error (so callers fall back to plaintext rather than
    /// silently dropping the token), and `remove` succeeds. The check runs
    /// before any backend is initialized, so this is platform-independent and
    /// green in a sandbox with no D-Bus/Keychain.
    #[test]
    fn disabled_keyring_store_is_no_backend() {
        temp_env::with_var(DISABLE_KEYRING_ENV_VAR, Some("true"), || {
            let store = KeyringStore::new(&Url::parse("https://hub.flox.dev").unwrap());

            assert_eq!(store.get().unwrap(), None);
            assert!(store.set(TOKEN).is_err());
            store.remove().unwrap();
        });
    }
}
