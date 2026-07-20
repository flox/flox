//! The plain-text token backend: `<config_dir>/flox.toml`, with an explicit
//! `0600` on write.

use std::path::{Path, PathBuf};

use flox_config::FLOX_CONFIG_FILE;

use super::{CredentialStore, CredentialStoreError};
use crate::commands::general::update_config;

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
/// [Config](flox_config::Config) value: it never sees a token contributed by
/// `/etc/flox.toml` or by `FLOX_FLOXHUB_TOKEN`. A missing file, a missing key,
/// or an empty string all resolve to `None`.
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

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::utils::credential_store::test_helpers::{TOKEN, write_flox_toml};

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
}
