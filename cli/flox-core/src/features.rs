use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Features {
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub beta: bool,
    #[serde(default)]
    pub auto_activate: bool,
}
