use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StartOrAttachResult {
    pub attach: bool,
    pub activation_state_dir: PathBuf,
    pub activation_id: String,
}
