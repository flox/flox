use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Full BLAKE3 hex output length for preference hashes.
/// Uses the same length as trust hashes for consistency, though preference
/// hashes are not security-sensitive (they key on path only).
const PREFERENCE_HASH_CHARS: usize = 64;

/// Status of auto-activation preference for a `.flox` environment path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreferenceStatus {
    Allowed,
    Denied,
    Unregistered,
}

/// Manages allowed/denied preference files that control whether the user
/// wants auto-activation for a `.flox` environment.
///
/// Unlike [`TrustManager`](crate::trust::TrustManager), preference is **not
/// content-sensitive**: the hash key is `blake3(absolute_path + "\n")` (path
/// only), so manifest changes do not affect the preference.
///
/// A single record per environment is maintained: `allow()` removes any
/// `denied/` file, and `deny()` removes any `allowed/` file.
#[derive(Clone, Debug)]
pub struct PreferenceManager {
    allowed_dir: PathBuf,
    denied_dir: PathBuf,
}

impl PreferenceManager {
    pub fn new(state_dir: impl AsRef<Path>) -> Self {
        let base = state_dir.as_ref().join("preference");
        Self {
            allowed_dir: base.join("allowed"),
            denied_dir: base.join("denied"),
        }
    }

    /// Check whether a `.flox` path has auto-activation allowed, denied,
    /// or unregistered.
    ///
    /// Denied takes priority over allowed (though in practice only one
    /// file should exist at a time).
    pub fn check(&self, dot_flox_path: impl AsRef<Path>) -> Result<PreferenceStatus> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);
        if self.denied_dir.join(&hash).exists() {
            return Ok(PreferenceStatus::Denied);
        }
        if self.allowed_dir.join(&hash).exists() {
            return Ok(PreferenceStatus::Allowed);
        }

        Ok(PreferenceStatus::Unregistered)
    }

    /// Allow auto-activation for a `.flox` path. Removes any existing
    /// denied file.
    pub fn allow(&self, dot_flox_path: impl AsRef<Path>) -> Result<()> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);

        // Remove any denied file first
        let denied_file = self.denied_dir.join(&hash);
        if denied_file.exists() {
            fs::remove_file(&denied_file)
                .with_context(|| format!("removing denied file {}", denied_file.display()))?;
        }

        fs::create_dir_all(&self.allowed_dir)
            .with_context(|| format!("creating {}", self.allowed_dir.display()))?;

        let allowed_file = self.allowed_dir.join(&hash);
        fs::write(&allowed_file, abs.display().to_string())
            .with_context(|| format!("writing allowed file {}", allowed_file.display()))?;

        Ok(())
    }

    /// Deny auto-activation for a `.flox` path. Removes any existing
    /// allowed file.
    pub fn deny(&self, dot_flox_path: impl AsRef<Path>) -> Result<()> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);

        // Remove any allowed file first
        let allowed_file = self.allowed_dir.join(&hash);
        if allowed_file.exists() {
            fs::remove_file(&allowed_file)
                .with_context(|| format!("removing allowed file {}", allowed_file.display()))?;
        }

        fs::create_dir_all(&self.denied_dir)
            .with_context(|| format!("creating {}", self.denied_dir.display()))?;

        let denied_file = self.denied_dir.join(&hash);
        fs::write(&denied_file, "")
            .with_context(|| format!("writing denied file {}", denied_file.display()))?;

        Ok(())
    }

    /// Compute the path hash: `blake3(absolute_path + "\n")`, truncated to
    /// [`PREFERENCE_HASH_CHARS`].
    fn path_hash(&self, abs_path: &Path) -> String {
        let input = format!("{}\n", abs_path.display());
        let mut hex = blake3::hash(input.as_bytes()).to_hex();
        hex.truncate(PREFERENCE_HASH_CHARS);
        hex.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal `.flox/env/manifest.toml` inside `dir`.
    fn create_dot_flox(dir: &Path) -> PathBuf {
        let dot_flox = dir.join(".flox");
        let env_dir = dot_flox.join("env");
        fs::create_dir_all(&env_dir).unwrap();
        fs::write(env_dir.join("manifest.toml"), "[install]").unwrap();
        dot_flox
    }

    #[test]
    fn unregistered_by_default() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        assert_eq!(
            mgr.check(&dot_flox).unwrap(),
            PreferenceStatus::Unregistered
        );
    }

    #[test]
    fn allow_then_check() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.allow(&dot_flox).unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Allowed);
    }

    #[test]
    fn deny_then_check() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.deny(&dot_flox).unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Denied);
    }

    #[test]
    fn allow_after_deny() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.deny(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Denied);

        mgr.allow(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Allowed);
    }

    #[test]
    fn deny_after_allow() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.allow(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Allowed);

        mgr.deny(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Denied);
    }

    #[test]
    fn not_content_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.allow(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Allowed);

        // Modify the manifest — preference should remain allowed
        let manifest_path = dot_flox.join("env").join("manifest.toml");
        fs::write(&manifest_path, "[install]\nhello = {}").unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Allowed);
    }
}
