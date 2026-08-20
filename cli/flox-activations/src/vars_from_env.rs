use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::attach_diff::{FLOX_ENV_DIRS_ADD_SBIN_VAR, FLOX_ENV_DIRS_VAR};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct VarsFromEnvironment {
    pub flox_env_dirs: Option<String>,
    /// The subset of `flox_env_dirs` whose environments add sbin to PATH.
    #[serde(default)]
    pub sbin_env_dirs: Option<String>,
    pub path: Option<String>,
    pub manpath: Option<String>,
    /// The environment as it stood before this activation applied anything,
    /// which is the baseline the `_FLOX_HOOK_DIFF` restored by
    /// `flox deactivate` is measured against.
    // TODO: should we drop the individual fields and just keep this one?
    pub full_env: HashMap<String, String>,
}

impl VarsFromEnvironment {
    /// Capture the pre-activation environment.
    ///
    /// Call this before mutating the process environment, so that the
    /// snapshot reflects the true pre-activation state.
    pub fn get() -> Result<Self> {
        // TODO(performance): is it faster to copy the entirety of env, or just get every environment variable we need?
        let all_vars: HashMap<String, String> = std::env::vars().collect();
        Ok(Self {
            flox_env_dirs: all_vars.get(FLOX_ENV_DIRS_VAR).cloned(),
            sbin_env_dirs: all_vars.get(FLOX_ENV_DIRS_ADD_SBIN_VAR).cloned(),
            path: all_vars.get("PATH").cloned(),
            manpath: all_vars.get("MANPATH").cloned(),
            full_env: all_vars,
        })
    }
}
