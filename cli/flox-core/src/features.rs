use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Features {
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub beta: bool,
    /// Zero-setup default environments: auto-create the default environment on
    /// first use and keep it synced with FloxHub (enable with
    /// `FLOX_FEATURES_AUTO_DEFAULT=true` or `flox config --set features.auto_default true`).
    #[serde(default)]
    pub auto_default: bool,
}
