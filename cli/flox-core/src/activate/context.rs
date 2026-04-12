use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shell_gen::ShellWithPath;
use uuid::Uuid;

pub use super::mode::ActivateMode;

/// Context needed to attach to a start of an environment
/// Note that store path is not included, as the executive needs to attach to
/// the latest ready store path when starting process-compose
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachCtx {
    /// The path to the environment symlink
    pub env: String,

    /// The cache path for the environment
    pub env_cache: PathBuf,

    /// The environment description
    pub env_description: String,

    /// Active environments tracking (JSON array)
    pub flox_active_environments: String,

    /// Prompt color 1
    pub prompt_color_1: String,

    /// Prompt color 2
    pub prompt_color_2: String,

    /// Prompt environments string
    pub flox_prompt_environments: String,

    /// Whether to set prompt
    pub set_prompt: bool,

    /// CUDA detection enabled
    pub flox_env_cuda_detection: String,

    /// Path to the interpreter (activate scripts)
    pub interpreter_path: PathBuf,
}

/// Additional context for project-based activations.
/// Includes project paths, logging, and service management.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachProjectCtx {
    /// The project path for the environment
    pub env_project: PathBuf,

    /// The path to the environment .flox directory
    pub dot_flox_path: PathBuf,

    /// Environment log directory
    pub flox_env_log_dir: PathBuf,

    /// Path to process-compose binary
    pub process_compose_bin: PathBuf,

    /// Services socket path
    pub flox_services_socket: PathBuf,

    /// Services to start with a new process-compose instance.
    /// When non-empty, flox-activations will start a new process-compose and start these services.
    pub services_to_start: Vec<String>,
}

/// Full activation context for activations.
/// For containers, project is None; for normal activations, it includes logging and services.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivateCtx {
    /// Store path for activation
    pub flox_activate_store_path: String,

    pub attach_ctx: AttachCtx,

    /// Project context for logging and services
    pub project_ctx: Option<AttachProjectCtx>,

    /// Base directory for this environment's activation state.
    pub activation_state_dir: PathBuf,

    /// The activation mode (dev or run)
    pub mode: ActivateMode,

    /// Path to the shell executable
    pub shell: ShellWithPath,

    /// The invocation type (interactive, command, etc.)
    /// None when determined at runtime (e.g., containers)
    pub invocation_type: Option<InvocationType>,

    /// Whether to clean up the context file after reading it.
    pub remove_after_reading: bool,

    /// The metrics UUID for this installation.
    /// When Some, Sentry is initialized with this user ID.
    /// When None, metrics are disabled and Sentry is not initialized.
    #[serde(default)]
    pub metrics_uuid: Option<Uuid>,

    /// Pre-computed hook registration code (shell-specific) for auto-activation
    /// in subshells. Set for Interactive invocations so that `flox activate`
    /// subshells preserve auto-activation functionality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_code: Option<String>,
}

/// Subset of activation context needed for auto-activation via `hook-env`.
///
/// Passed as JSON to `flox-activations auto-start` to register the shell PID,
/// spawn the executive, and run lifecycle hooks.
///
/// Composes `AttachCtx` and `AttachProjectCtx` so that `auto_start` can pass
/// them directly to `spawn_executive` and `assemble_auto_activate_command`
/// without reconstructing them from flat fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStartCtx {
    pub attach_ctx: AttachCtx,
    pub project_ctx: AttachProjectCtx,

    /// Nix store path for the built environment
    pub store_path: String,

    /// Base directory for this environment's activation state
    pub activation_state_dir: PathBuf,

    /// The activation mode (dev or run)
    pub mode: ActivateMode,

    /// The metrics UUID for Sentry user identification.
    #[serde(default)]
    pub metrics_uuid: Option<Uuid>,
}

/// Result returned on stdout (as JSON) by `flox-activations auto-start`.
/// Parsed by `hook-env` to merge on-activate env diff and track state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoStartResult {
    pub status: String,
    pub start_id: String,
    pub is_new: bool,
    /// Environment variable changes produced by on-activate hooks (only when is_new=true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hook_env_diff: Option<crate::hook_state::OnActivateEnvDiff>,
    /// Start state directory path (only when is_new=true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_state_dir: Option<String>,
}

#[derive(Clone, Debug, Deserialize, derive_more::Display, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InvocationType {
    #[display("inplace")]
    InPlace,
    #[display("interactive")]
    Interactive,
    #[display("command")]
    ShellCommand(String),
    #[display("execcommand")]
    ExecCommand(Vec<String>),
}
