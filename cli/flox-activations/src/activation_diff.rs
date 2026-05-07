use std::collections::HashMap;
use std::io::{Read, Write};

use anyhow::Result;
use base64::Engine as _;
use serde::{Deserialize, Serialize};

pub const FLOX_HOOK_DIFF_VAR: &str = "_FLOX_HOOK_DIFF";

/// The diff between the pre-activation shell environment and the intended
/// post-activation environment, captured at attach time.
///
/// Each category stores the *original* value (for deactivation purposes),
/// except for `added` which stores the new value (since there is no original).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiffSerializer {
    /// Vars newly set by activation (stores new value).
    pub added: HashMap<String, String>,
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

    #[test]
    fn encode_decode_roundtrip() {
        let original = DiffSerializer {
            added: make_env(&[("NEW_VAR", "new_value")]),
            modified: make_env(&[("MOD_VAR", "original_value")]),
            removed: make_env(&[("REM_VAR", "removed_value")]),
        };

        let encoded = original.encode().expect("encode should succeed");
        let decoded = DiffSerializer::decode(&encoded).expect("decode should succeed");

        assert_eq!(original, decoded);
    }
}
