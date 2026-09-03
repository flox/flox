use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Features {
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub beta: bool,
    /// Arms execution of `[plugin-hooks]` declarations (session-wrap and
    /// friends). Deliberately separate from `beta`: enabling beta to try
    /// subcommand extensions must not silently arm session handoff.
    #[serde(default)]
    pub plugin_hooks: bool,
}
