use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_activations::attach_diff::diff_serializer::{DiffSerializer, FLOX_HOOK_DIFF_VAR};

/// Decode the `_FLOX_HOOK_DIFF` environment variable into pretty-printed JSON.
///
/// `_FLOX_HOOK_DIFF` is set during activation as zlib-compressed, base64url
/// JSON of the environment diff used to restore the shell on deactivation.
/// This hidden command makes that opaque value readable for debugging.
#[derive(Bpaf, Clone, Debug)]
pub struct DecodeHookDiff {}

impl DecodeHookDiff {
    pub fn handle(self) -> Result<()> {
        let encoded = std::env::var(FLOX_HOOK_DIFF_VAR)
            .context(format!("{FLOX_HOOK_DIFF_VAR} not set in environment"))?;
        let diff = DiffSerializer::decode(&encoded)
            .context(format!("Failed to decode {FLOX_HOOK_DIFF_VAR}"))?;
        println!("{}", serde_json::to_string_pretty(&diff)?);
        Ok(())
    }
}
