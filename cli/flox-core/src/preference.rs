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
    Enabled,
    Disabled,
    Unregistered,
}

/// Manages enabled/disabled preference files that control whether the user
/// wants auto-activation for a `.flox` environment.
///
/// Unlike [`TrustManager`](crate::trust::TrustManager), preference is **not
/// content-sensitive**: the hash key is `blake3(absolute_path + "\n")` (path
/// only), so manifest changes do not affect the preference.
///
/// A single record per environment is maintained: `enable()` removes any
/// `disabled/` file, and `disable()` removes any `enabled/` file.
#[derive(Clone, Debug)]
pub struct PreferenceManager {
    enabled_dir: PathBuf,
    disabled_dir: PathBuf,
}

impl PreferenceManager {
    pub fn new(state_dir: impl AsRef<Path>) -> Self {
        let base = state_dir.as_ref().join("preference");
        Self {
            enabled_dir: base.join("enabled"),
            disabled_dir: base.join("disabled"),
        }
    }

    /// Check whether a `.flox` path has auto-activation enabled, disabled,
    /// or unregistered.
    ///
    /// Disabled takes priority over enabled (though in practice only one
    /// file should exist at a time).
    pub fn check(&self, dot_flox_path: impl AsRef<Path>) -> Result<PreferenceStatus> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);
        if self.disabled_dir.join(&hash).exists() {
            return Ok(PreferenceStatus::Disabled);
        }
        if self.enabled_dir.join(&hash).exists() {
            return Ok(PreferenceStatus::Enabled);
        }

        Ok(PreferenceStatus::Unregistered)
    }

    /// Enable auto-activation for a `.flox` path. Removes any existing
    /// disabled file.
    pub fn enable(&self, dot_flox_path: impl AsRef<Path>) -> Result<()> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);

        // Remove any disabled file first
        let disabled_file = self.disabled_dir.join(&hash);
        if disabled_file.exists() {
            fs::remove_file(&disabled_file)
                .with_context(|| format!("removing disabled file {}", disabled_file.display()))?;
        }

        fs::create_dir_all(&self.enabled_dir)
            .with_context(|| format!("creating {}", self.enabled_dir.display()))?;

        let enabled_file = self.enabled_dir.join(&hash);
        fs::write(&enabled_file, abs.display().to_string())
            .with_context(|| format!("writing enabled file {}", enabled_file.display()))?;

        Ok(())
    }

    /// Disable auto-activation for a `.flox` path. Removes any existing
    /// enabled file.
    pub fn disable(&self, dot_flox_path: impl AsRef<Path>) -> Result<()> {
        let dot_flox_path = dot_flox_path.as_ref();
        let abs = fs::canonicalize(dot_flox_path)
            .with_context(|| format!("canonicalizing {}", dot_flox_path.display()))?;

        let hash = self.path_hash(&abs);

        // Remove any enabled file first
        let enabled_file = self.enabled_dir.join(&hash);
        if enabled_file.exists() {
            fs::remove_file(&enabled_file)
                .with_context(|| format!("removing enabled file {}", enabled_file.display()))?;
        }

        fs::create_dir_all(&self.disabled_dir)
            .with_context(|| format!("creating {}", self.disabled_dir.display()))?;

        let disabled_file = self.disabled_dir.join(&hash);
        fs::write(&disabled_file, "")
            .with_context(|| format!("writing disabled file {}", disabled_file.display()))?;

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
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Unregistered);
    }

    #[test]
    fn enable_then_check() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.enable(&dot_flox).unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Enabled);
    }

    #[test]
    fn disable_then_check() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.disable(&dot_flox).unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Disabled);
    }

    #[test]
    fn enable_after_disable() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.disable(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Disabled);

        mgr.enable(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Enabled);
    }

    #[test]
    fn disable_after_enable() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.enable(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Enabled);

        mgr.disable(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Disabled);
    }

    #[test]
    fn not_content_sensitive() {
        let tmp = tempfile::tempdir().unwrap();
        let state_dir = tmp.path().join("state");
        let dot_flox = create_dot_flox(tmp.path());

        let mgr = PreferenceManager::new(&state_dir);
        mgr.enable(&dot_flox).unwrap();
        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Enabled);

        // Modify the manifest — preference should remain enabled
        let manifest_path = dot_flox.join("env").join("manifest.toml");
        fs::write(&manifest_path, "[install]\nhello = {}").unwrap();

        assert_eq!(mgr.check(&dot_flox).unwrap(), PreferenceStatus::Enabled);
    }
}
