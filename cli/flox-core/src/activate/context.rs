use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::shell::ShellWithPath;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateCtx {
    // Command arguments (from command.arg() calls in cli/flox/src/commands/activate.rs:437-462)
    /// The path to the environment symlink
    pub env: String,

    /// The project path for the environment
    pub env_project: Option<PathBuf>,

    /// The cache path for the environment
    pub env_cache: PathBuf,

    /// The environment description
    pub env_description: String,

    /// The activation mode (dev or run)
    pub mode: String,

    /// Path to the watchdog binary
    pub watchdog_bin: Option<PathBuf>,

    /// Path to the shell executable
    pub shell: ShellWithPath,

    // Environment variable exports (from exports HashMap in cli/flox/src/commands/activate.rs:332-428)
    /// Active environments tracking
    pub flox_active_environments: String,

    /// Environment log directory
    pub flox_env_log_dir: Option<String>,

    /// Prompt color 1
    pub prompt_color_1: String,

    /// Prompt color 2
    pub prompt_color_2: String,

    /// Prompt environments string
    pub flox_prompt_environments: String,

    /// Whether to set prompt
    pub set_prompt: bool,

    /// Store path for activation
    pub flox_activate_store_path: String,

    /// Runtime directory
    pub flox_runtime_dir: String,

    /// Services to start (JSON array)
    pub flox_services_to_start: Option<String>,

    /// CUDA detection enabled
    pub flox_env_cuda_detection: String,

    /// Whether to start services
    pub flox_activate_start_services: bool,

    /// Services socket path
    pub flox_services_socket: Option<String>,

    // Info needed to run the activate script
    pub interpreter_path: PathBuf,
    pub invocation_type: Option<InvocationType>,
    pub run_args: Vec<String>,

    /// Whether to clean up the context file after reading it.
    pub remove_after_reading: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum InvocationType {
    InPlace,
    Interactive,
    Command,
}
