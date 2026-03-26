use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// Serialize a value to JSON, zlib compress, then base64url encode (no padding).
fn compress_to_base64<T: Serialize>(val: &T) -> Result<String> {
    let json = serde_json::to_string(val).context("failed to serialize to JSON")?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(json.as_bytes())
        .context("failed to zlib compress")?;
    let compressed = encoder
        .finish()
        .context("failed to finish zlib compression")?;
    Ok(URL_SAFE_NO_PAD.encode(&compressed))
}

/// Deserialize from base64url encoded, zlib compressed JSON.
fn decompress_from_base64<T: DeserializeOwned>(encoded: &str) -> Result<T> {
    let compressed = URL_SAFE_NO_PAD
        .decode(encoded)
        .context("failed to base64url decode")?;
    let mut decoder = ZlibDecoder::new(&compressed[..]);
    let mut json = String::new();
    decoder
        .read_to_string(&mut json)
        .context("failed to zlib decompress")?;
    serde_json::from_str(&json).context("failed to deserialize from JSON")
}

pub const HOOK_VAR_DIFF: &str = "_FLOX_HOOK_DIFF";
pub const HOOK_VAR_DIRS: &str = "_FLOX_HOOK_DIRS";
pub const HOOK_VAR_WATCHES: &str = "_FLOX_HOOK_WATCHES";
pub const HOOK_VAR_SUPPRESSED: &str = "_FLOX_HOOK_SUPPRESSED";
pub const HOOK_VAR_NOTIFIED: &str = "_FLOX_HOOK_NOTIFIED";
pub const HOOK_VAR_CWD: &str = "_FLOX_HOOK_CWD";
pub const HOOK_VAR_ACTIVATIONS: &str = "_FLOX_HOOK_ACTIVATIONS";

/// Environment variable changes produced by on-activate hooks.
/// Passed from `flox-activations` back to `hook-env` via `AutoStartResult`,
/// and cached in `ActivationInfo` for cd-away-and-back without re-running hooks.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct OnActivateEnvDiff {
    pub additions: HashMap<String, String>,
    pub deletions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct HookDiff {
    pub additions: HashMap<String, String>,
    pub modifications: HashMap<String, String>,
    pub deletions: HashMap<String, String>,
}

impl HookDiff {
    /// Compute the diff between a pristine environment and a new environment.
    ///
    /// - `additions`: keys in `new_env` but not in `pristine`
    /// - `modifications`: keys in both with different values (stores the ORIGINAL value)
    /// - `deletions`: keys in `pristine` but not in `new_env` (stores the ORIGINAL value)
    pub fn compute(pristine: &HashMap<String, String>, new_env: &HashMap<String, String>) -> Self {
        let mut additions = HashMap::new();
        let mut modifications = HashMap::new();
        let mut deletions = HashMap::new();

        for (key, new_val) in new_env {
            match pristine.get(key) {
                Some(orig_val) if orig_val != new_val => {
                    modifications.insert(key.clone(), orig_val.clone());
                },
                None => {
                    additions.insert(key.clone(), new_val.clone());
                },
                _ => {},
            }
        }

        for (key, orig_val) in pristine {
            if !new_env.contains_key(key) {
                deletions.insert(key.clone(), orig_val.clone());
            }
        }

        Self {
            additions,
            modifications,
            deletions,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.additions.is_empty() && self.modifications.is_empty() && self.deletions.is_empty()
    }

    /// Serialize to JSON, zlib compress, then base64url encode (no padding).
    pub fn serialize(&self) -> Result<String> {
        compress_to_base64(self)
    }

    /// Deserialize from base64url encoded, zlib compressed JSON.
    /// An empty string returns the default (empty) HookDiff.
    pub fn deserialize(encoded: &str) -> Result<Self> {
        if encoded.is_empty() {
            return Ok(Self::default());
        }
        decompress_from_base64(encoded)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WatchEntry {
    pub path: PathBuf,
    pub mtime: Option<u64>,
}

/// Per-environment activation info tracked by hook-env.
/// Maps dot_flox_path to the activation state directory and store path
/// so that auto-detach knows where to find the activation state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct ActivationTracking {
    pub entries: HashMap<PathBuf, ActivationInfo>,
    /// Cache of activation info for environments the user has cd'd away from.
    /// Preserves on_activate_diff so cd-back doesn't re-run hooks.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub detached_cache: HashMap<PathBuf, ActivationInfo>,
}

/// Metadata for a single auto-activated environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivationInfo {
    /// Base directory for this environment's activation state
    pub activation_state_dir: PathBuf,
    /// Nix store path for the built environment
    pub store_path: String,
    /// Start state directory for the current activation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_state_dir: Option<PathBuf>,
    /// Cached on-activate hook env diff (avoids re-running hooks on cd-back)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_activate_diff: Option<OnActivateEnvDiff>,
}

impl ActivationTracking {
    /// Serialize to JSON, zlib compress, then base64url encode (no padding).
    pub fn serialize(&self) -> Result<String> {
        if self.entries.is_empty() && self.detached_cache.is_empty() {
            return Ok(String::new());
        }
        compress_to_base64(self)
    }

    /// Deserialize from base64url encoded, zlib compressed JSON.
    /// An empty string returns the default (empty) ActivationTracking.
    pub fn deserialize(encoded: &str) -> Result<Self> {
        if encoded.is_empty() {
            return Ok(Self::default());
        }
        decompress_from_base64(encoded)
    }
}

#[derive(Debug, Clone)]
pub struct HookState {
    pub diff: HookDiff,
    pub active_dirs: Vec<PathBuf>,
    pub watches: Vec<WatchEntry>,
    pub suppressed_dirs: Vec<PathBuf>,
    pub notified_dirs: Vec<PathBuf>,
    pub last_cwd: Option<PathBuf>,
    pub activation_tracking: ActivationTracking,
}

impl HookState {
    /// Read hook state from environment variables.
    pub fn from_env() -> Result<Self> {
        let diff_str = std::env::var(HOOK_VAR_DIFF).unwrap_or_default();
        let diff = HookDiff::deserialize(&diff_str).context("failed to parse _FLOX_HOOK_DIFF")?;

        let dirs_str = std::env::var(HOOK_VAR_DIRS).unwrap_or_default();
        let active_dirs = Self::parse_path_list(&dirs_str);

        let watches_str = std::env::var(HOOK_VAR_WATCHES).unwrap_or_default();
        let watches: Vec<WatchEntry> = if watches_str.is_empty() {
            Vec::new()
        } else {
            serde_json::from_str(&watches_str).context("failed to parse _FLOX_HOOK_WATCHES")?
        };

        let suppressed_str = std::env::var(HOOK_VAR_SUPPRESSED).unwrap_or_default();
        let suppressed_dirs = Self::parse_path_list(&suppressed_str);

        let notified_str = std::env::var(HOOK_VAR_NOTIFIED).unwrap_or_default();
        let notified_dirs = Self::parse_path_list(&notified_str);

        let last_cwd = std::env::var(HOOK_VAR_CWD)
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);

        let activations_str = std::env::var(HOOK_VAR_ACTIVATIONS).unwrap_or_default();
        let activation_tracking = ActivationTracking::deserialize(&activations_str)
            .context("failed to parse _FLOX_HOOK_ACTIVATIONS")?;

        Ok(Self {
            diff,
            active_dirs,
            watches,
            suppressed_dirs,
            notified_dirs,
            last_cwd,
            activation_tracking,
        })
    }

    /// Check if any watched file has changed by comparing current mtime to recorded mtime.
    pub fn watches_changed(&self) -> bool {
        for entry in &self.watches {
            let current_mtime = std::fs::metadata(&entry.path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs());
            if current_mtime != entry.mtime {
                return true;
            }
        }
        false
    }

    /// Format a list of paths as a colon-separated string.
    pub fn format_path_list(paths: &[PathBuf]) -> String {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Parse a colon-separated string into a list of paths.
    /// An empty string returns an empty list.
    pub fn parse_path_list(s: &str) -> Vec<PathBuf> {
        if s.is_empty() {
            return Vec::new();
        }
        s.split(':').map(PathBuf::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_addition() {
        let pristine = HashMap::new();
        let mut new_env = HashMap::new();
        new_env.insert("FOO".to_string(), "bar".to_string());

        let diff = HookDiff::compute(&pristine, &new_env);
        assert_eq!(diff, HookDiff {
            additions: HashMap::from([("FOO".to_string(), "bar".to_string())]),
            modifications: HashMap::new(),
            deletions: HashMap::new(),
        });
    }

    #[test]
    fn test_compute_modification() {
        let mut pristine = HashMap::new();
        pristine.insert("FOO".to_string(), "old".to_string());
        let mut new_env = HashMap::new();
        new_env.insert("FOO".to_string(), "new".to_string());

        let diff = HookDiff::compute(&pristine, &new_env);
        assert_eq!(diff, HookDiff {
            additions: HashMap::new(),
            modifications: HashMap::from([("FOO".to_string(), "old".to_string())]),
            deletions: HashMap::new(),
        });
    }

    #[test]
    fn test_compute_deletion() {
        let mut pristine = HashMap::new();
        pristine.insert("FOO".to_string(), "bar".to_string());
        let new_env = HashMap::new();

        let diff = HookDiff::compute(&pristine, &new_env);
        assert_eq!(diff, HookDiff {
            additions: HashMap::new(),
            modifications: HashMap::new(),
            deletions: HashMap::from([("FOO".to_string(), "bar".to_string())]),
        });
    }

    #[test]
    fn test_compute_no_change() {
        let mut pristine = HashMap::new();
        pristine.insert("FOO".to_string(), "bar".to_string());
        let mut new_env = HashMap::new();
        new_env.insert("FOO".to_string(), "bar".to_string());

        let diff = HookDiff::compute(&pristine, &new_env);
        assert_eq!(diff, HookDiff::default());
    }

    #[test]
    fn test_serialize_deserialize_roundtrip() {
        let diff = HookDiff {
            additions: HashMap::from([("NEW".to_string(), "val".to_string())]),
            modifications: HashMap::from([("MOD".to_string(), "orig".to_string())]),
            deletions: HashMap::from([("DEL".to_string(), "gone".to_string())]),
        };

        let encoded = diff.serialize().unwrap();
        let decoded = HookDiff::deserialize(&encoded).unwrap();
        assert_eq!(decoded, diff);
    }

    #[test]
    fn test_deserialize_empty_string() {
        let diff = HookDiff::deserialize("").unwrap();
        assert_eq!(diff, HookDiff::default());
    }

    #[test]
    fn test_path_list_roundtrip() {
        let paths = vec![
            PathBuf::from("/home/user/.flox/env1"),
            PathBuf::from("/home/user/.flox/env2"),
        ];
        let serialized = HookState::format_path_list(&paths);
        let deserialized = HookState::parse_path_list(&serialized);
        assert_eq!(deserialized, paths);
    }

    #[test]
    fn test_parse_empty_path_list() {
        let paths = HookState::parse_path_list("");
        assert_eq!(paths, Vec::<PathBuf>::new());
    }

    #[test]
    fn test_activation_tracking_serialize_deserialize_roundtrip() {
        let mut tracking = ActivationTracking::default();
        tracking
            .entries
            .insert(PathBuf::from("/home/user/project/.flox"), ActivationInfo {
                activation_state_dir: PathBuf::from(
                    "/run/user/1000/flox/activations/abc12345-project",
                ),
                store_path: "/nix/store/abc-env".to_string(),
                start_state_dir: None,
                on_activate_diff: None,
            });

        let encoded = tracking.serialize().unwrap();
        let decoded = ActivationTracking::deserialize(&encoded).unwrap();
        assert_eq!(decoded, tracking);
    }

    #[test]
    fn test_activation_tracking_deserialize_empty_string() {
        let tracking = ActivationTracking::deserialize("").unwrap();
        assert_eq!(tracking, ActivationTracking::default());
    }

    #[test]
    fn test_activation_tracking_serialize_empty_returns_empty_string() {
        let tracking = ActivationTracking::default();
        let encoded = tracking.serialize().unwrap();
        assert_eq!(encoded, "");
    }
}
