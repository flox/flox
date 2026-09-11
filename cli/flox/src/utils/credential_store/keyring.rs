//! The OS-native encrypted credential backend.
//!
//! On macOS the login Keychain is driven through the Apple-signed
//! `security(1)` tool ([super::security_cli]) rather than the Security
//! framework: the Keychain trusts the *application* that created an item,
//! keyed by its code signature, and a nix-built `flox` is ad-hoc signed per
//! build, so a native read prompted again after every upgrade and rebuild
//! even after "Always Allow" (DEV-290). Every other platform talks to the
//! keyring via the `keyring` v4 crates (Linux: Secret Service over D-Bus).

#[cfg(not(target_os = "macos"))]
use std::sync::Once;

use url::Url;

#[cfg(target_os = "macos")]
use super::security_cli::{SecurityCli, SecurityCliError};
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
/// returns [keyring_core::Error::NoDefaultStore], which [KeyringStore::remove]
/// treats as a no-backend condition.
#[cfg(not(target_os = "macos"))]
fn register_default_store() {
    static SET_CREDENTIAL_STORE: Once = Once::new();
    SET_CREDENTIAL_STORE.call_once(|| {
        #[cfg(target_os = "linux")]
        {
            if let Ok(store) = zbus_secret_service_keyring_store::Store::new() {
                keyring_core::set_default_store(store);
            }
        }
    });
}

/// OS-native encrypted credential storage (macOS Keychain / Linux Secret
/// Service).
///
/// The entry is keyed by [KEYRING_SERVICE] plus the FloxHub base URL as the
/// account, so distinct FloxHub instances do not collide.
#[derive(Debug, Clone)]
pub(super) struct KeyringStore {
    account: String,
}

impl KeyringStore {
    pub(super) fn new(floxhub_url: &Url) -> Self {
        Self {
            account: floxhub_url.as_str().to_string(),
        }
    }
}

#[cfg(target_os = "macos")]
impl KeyringStore {
    fn security(&self) -> SecurityCli {
        SecurityCli::new(KEYRING_SERVICE, &self.account, None)
    }

    fn get_from_backend(&self) -> Result<Option<String>, CredentialStoreError> {
        Ok(self.security().get()?)
    }

    fn set_in_backend(&self, token: &str) -> Result<(), CredentialStoreError> {
        Ok(self.security().set(token)?)
    }

    fn remove_from_backend(&self) -> Result<(), CredentialStoreError> {
        Ok(self.security().remove()?)
    }
}

#[cfg(not(target_os = "macos"))]
impl KeyringStore {
    fn entry(&self) -> Result<keyring_core::Entry, CredentialStoreError> {
        register_default_store();
        Ok(keyring_core::Entry::new(KEYRING_SERVICE, &self.account)?)
    }

    fn get_from_backend(&self) -> Result<Option<String>, CredentialStoreError> {
        match self.entry()?.get_password() {
            Ok(password) => Ok(Some(password)),
            Err(keyring_core::Error::NoEntry) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn set_in_backend(&self, token: &str) -> Result<(), CredentialStoreError> {
        Ok(self.entry()?.set_password(token)?)
    }

    fn remove_from_backend(&self) -> Result<(), CredentialStoreError> {
        match self.entry()?.delete_credential() {
            // Idempotent: a missing entry is not a failure.
            Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

impl CredentialStore for KeyringStore {
    fn get(&self) -> Result<Option<String>, CredentialStoreError> {
        // Disabled keyring: behave as a no-backend box. Return `Ok(None)` (not
        // an error) so this path is deterministic on a developer's keyring-
        // capable machine. Checked before any backend is touched, so no OS
        // unlock prompt is triggered.
        if keyring_disabled() {
            return Ok(None);
        }
        self.get_from_backend()
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
        self.set_in_backend(token)
    }

    fn remove(&self) -> Result<(), CredentialStoreError> {
        // Disabled keyring: nothing of ours is stored, so removal is a no-op
        // success — logout must still succeed.
        if keyring_disabled() {
            return Ok(());
        }
        // Best-effort across machines with no keyring: when no usable backend
        // is available there is nothing of ours stored there, so logout still
        // succeeds. Any other failure is surfaced so logout does not falsely
        // claim success.
        match self.remove_from_backend() {
            // No Secret Service (or equivalent) to talk to.
            #[cfg(not(target_os = "macos"))]
            Err(CredentialStoreError::Keyring(
                keyring_core::Error::NoDefaultStore
                | keyring_core::Error::PlatformFailure(_)
                | keyring_core::Error::NoStorageAccess(_),
            )) => Ok(()),
            // No `security` tool to run.
            #[cfg(target_os = "macos")]
            Err(CredentialStoreError::SecurityTool(SecurityCliError::Spawn { .. })) => Ok(()),
            other => other,
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
