use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use anyhow::Result;
use base64::Engine as _;
use flox_core::activate::vars::FLOX_ACTIVE_ENVIRONMENTS_VAR;
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use shell_gen::Statement;

use crate::attach_diff::{set_exported_unexpanded, unset};

pub const FLOX_HOOK_DIFF_VAR: &str = "_FLOX_HOOK_DIFF";

/// The diff between the pre-activation shell environment and the intended
/// post-activation environment, captured at attach time.
///
/// `modified` and `removed` store the *original* value so deactivation can
/// restore it. `added` stores only the name: the var did not exist before, so
/// deactivation will unset it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffSerializer {
    /// Vars newly set by activation.
    pub added: HashSet<String>,
    /// Vars whose value will change (stores *original* value).
    pub modified: HashMap<String, String>,
    /// Vars that will be unset (stores *original* value).
    pub removed: HashMap<String, String>,
}

impl DiffSerializer {
    /// Serialize to zlib-compressed base64url JSON.
    pub fn encode(&self) -> Result<String> {
        let json = serde_json::to_vec(self)?;
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&json)?;
        let compressed = encoder.finish()?;
        Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&compressed))
    }

    /// Deserialize from zlib-compressed base64url JSON.
    ///
    /// Used for deactivation to restore the original environment.
    pub fn decode(encoded: &str) -> Result<Self> {
        let compressed = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)?;
        let mut decoder = flate2::read::ZlibDecoder::new(&compressed[..]);
        let mut json = Vec::new();
        decoder.read_to_end(&mut json)?;
        Ok(serde_json::from_slice(&json)?)
    }

    /// Generates shell statements to restore the environment to its pre-activation state:
    /// - Unsets variables that were added during activation
    /// - Restores original values for variables that were modified
    /// - Restores variables that were removed during activation
    ///
    /// For in-place activations, `_FLOX_HOOK_DIFF` is included in `added`
    /// (first activation) or `modified` (nested), so the loops above handle it
    /// — restoring the outer value rather than clearing it. For non-in-place
    /// (subshell) activations it is not in the diff, so it is unset
    /// unconditionally here instead.
    pub(crate) fn generate_deactivation_statements(&self) -> Vec<Statement> {
        let mut stmts = Vec::new();
        // Unset variables that were added during activation
        for var_name in self.added.iter().sorted() {
            stmts.push(unset(var_name));
        }

        // Restore variables that were modified during activation
        for (var_name, original_value) in self.modified.iter().sorted_by_key(|(k, _)| *k) {
            stmts.push(set_exported_unexpanded(var_name, original_value));
        }

        // Restore variables that were removed during activation
        for (var_name, original_value) in self.removed.iter().sorted_by_key(|(k, _)| *k) {
            stmts.push(set_exported_unexpanded(var_name, original_value));
        }

        // Non-in-place activations don't include this in the diff (set on the
        // subprocess directly); in-place activations already handle it above.
        if !self.added.contains(FLOX_HOOK_DIFF_VAR)
            && !self.modified.contains_key(FLOX_HOOK_DIFF_VAR)
        {
            stmts.push(unset(FLOX_HOOK_DIFF_VAR));
        }

        stmts
    }

    /// True when restoring this diff would leave `_FLOX_ACTIVE_ENVIRONMENTS`
    /// empty or unset — i.e., this deactivate is undoing the outermost
    /// activation. Per-shell teardown (hashing, FPATH, precmd hook removal)
    /// is gated on this.
    ///
    /// Activation always sets `_FLOX_ACTIVE_ENVIRONMENTS` via
    /// `single_set_envs`, so the diff will have it in `added` (first
    /// activation, no prior value) or `modified` (nested, with the prior
    /// outer value). The trailing `false` covers the logically-impossible
    /// case that activation didn't set it.
    pub(crate) fn is_outermost_deactivate(&self) -> bool {
        if self.added.contains(FLOX_ACTIVE_ENVIRONMENTS_VAR) {
            return true;
        }
        if let Some(active_environments) = self.modified.get(FLOX_ACTIVE_ENVIRONMENTS_VAR) {
            return active_environments.is_empty();
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_env(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn make_keys(keys: &[&str]) -> HashSet<String> {
        keys.iter().map(|k| k.to_string()).collect()
    }

    #[test]
    fn encode_decode_roundtrip() {
        let original = DiffSerializer {
            added: make_keys(&["NEW_VAR"]),
            modified: make_env(&[("MOD_VAR", "original_value")]),
            removed: make_env(&[("REM_VAR", "removed_value")]),
        };

        let encoded = original.encode().expect("encode should succeed");
        let decoded = DiffSerializer::decode(&encoded).expect("decode should succeed");

        assert_eq!(original, decoded);
    }

    fn diff_with(
        added: &[&str],
        modified: &[(&str, &str)],
        removed: &[(&str, &str)],
    ) -> DiffSerializer {
        DiffSerializer {
            added: make_keys(added),
            modified: make_env(modified),
            removed: make_env(removed),
        }
    }

    #[test]
    fn outermost_on_first_activation() {
        // First activation: pre-activation env had no `_FLOX_ACTIVE_ENVIRONMENTS`,
        // so activation puts it in `added`. Post-restore: unset → outermost.
        let diff = diff_with(&[FLOX_ACTIVE_ENVIRONMENTS_VAR], &[], &[]);
        assert!(diff.is_outermost_deactivate());
    }

    #[test]
    fn not_outermost_on_nested_activation() {
        // Nested activation: pre-activation env already had an outer
        // `_FLOX_ACTIVE_ENVIRONMENTS`, so activation puts it in `modified`
        // with that outer value. Post-restore: outer value → not outermost.
        let diff = diff_with(&[], &[(FLOX_ACTIVE_ENVIRONMENTS_VAR, "/outer/env")], &[]);
        assert!(!diff.is_outermost_deactivate());
    }

    #[test]
    fn outermost_when_modified_to_empty() {
        // Pre-activation env had `_FLOX_ACTIVE_ENVIRONMENTS=""` (set but
        // empty). Activation puts it in `modified` with empty value.
        // Post-restore: empty string → still outermost.
        let diff = diff_with(&[], &[(FLOX_ACTIVE_ENVIRONMENTS_VAR, "")], &[]);
        assert!(diff.is_outermost_deactivate());
    }
}
