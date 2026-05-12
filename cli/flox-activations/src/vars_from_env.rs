use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::attach_diff::FLOX_ENV_DIRS_VAR;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VarsFromEnvironment {
    pub flox_env_dirs: Option<String>,
    pub path: Option<String>,
    pub manpath: Option<String>,
    /// Full environment snapshot for activation diff computation.
    /// Only populated when auto_activate is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_env: Option<HashMap<String, String>>,
}

impl VarsFromEnvironment {
    pub fn get() -> Result<Self> {
        let flox_env_dirs = std::env::var(FLOX_ENV_DIRS_VAR).ok();
        let path = std::env::var("PATH").ok();
        let manpath = std::env::var("MANPATH").ok();

        Ok(Self {
            flox_env_dirs,
            path,
            manpath,
            full_env: None,
        })
    }

    /// Capture path-related vars plus full env snapshot.
    /// Used when auto_activate is enabled.
    pub fn get_with_snapshot() -> Result<Self> {
        // TODO(performance): is it faster to copy the entirety of env, or just get every environment variable we need?
        let all_vars: HashMap<String, String> = std::env::vars().collect();
        Ok(Self {
            flox_env_dirs: all_vars.get(FLOX_ENV_DIRS_VAR).cloned(),
            path: all_vars.get("PATH").cloned(),
            manpath: all_vars.get("MANPATH").cloned(),
            full_env: Some(all_vars),
        })
    }
}
