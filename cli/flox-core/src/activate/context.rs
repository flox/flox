use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use shell_gen::ShellWithPath;
use uuid::Uuid;

pub use super::mode::ActivateMode;
pub use super::sandbox_mode::SandboxMode;

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

    /// The sandbox mode for this activation.
    /// Absent in older context files, which deserialize to `Off`.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,

    /// Hostname of flox's own metrics endpoint, seeded into the sandbox
    /// network policy as a visible default-seed grant so the CLI's telemetry
    /// flush is not reported (and blocked) as workload egress. `None` when
    /// the user disabled metrics. Absent in older context files.
    #[serde(default)]
    pub metrics_host: Option<String>,
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

    /// Whether to include auto-activation hook code in the activation
    /// output. Gated behind the auto_activate feature flag.
    #[serde(default)]
    pub auto_activate: bool,

    /// Passthrough for config.disable_hook.unwrap_or(false)
    #[serde(default)]
    pub disable_hook: bool,

    /// Path to the flox binary, used for generating hook code.
    #[serde(default)]
    pub flox_bin: String,

    /// Controls how the fish shell hook responds to directory changes.
    #[serde(default)]
    pub auto_activate_fish_mode: Option<AutoActivateFishMode>,

    /// The sandbox mode for this activation.
    /// Absent in older context files, which deserialize to `Off`.
    #[serde(default)]
    pub sandbox_mode: SandboxMode,
}

/// Fish shell hook mode, matching direnv's `direnv_fish_mode` values.
#[derive(
    Clone, Copy, Debug, Default, Deserialize, derive_more::Display, Serialize, PartialEq, Eq,
)]
#[serde(rename_all = "snake_case")]
pub enum AutoActivateFishMode {
    /// Evaluate on prompt and immediately on PWD change (default).
    #[default]
    #[display("eval_on_arrow")]
    EvalOnArrow,
    /// Evaluate on prompt; defer PWD-change evaluation until before the next command.
    #[display("eval_after_arrow")]
    EvalAfterArrow,
    /// Evaluate on prompt only; ignore directory changes.
    #[display("disable_arrow")]
    DisableArrow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InvocationType {
    InPlace,
    Interactive,
    ShellCommand(String),
    ExecCommand(Vec<String>),
}

impl InvocationType {
    pub fn is_in_place(&self) -> bool {
        matches!(self, Self::InPlace)
    }

    pub fn kind(&self) -> InvocationKind {
        match self {
            Self::InPlace => InvocationKind::InPlace,
            Self::Interactive => InvocationKind::Interactive,
            Self::ShellCommand(_) => InvocationKind::ShellCommand,
            Self::ExecCommand(_) => InvocationKind::ExecCommand,
        }
    }
}

/// Drops the user command wrapped by `ShellCommand` and `ExecCommand` so we can
/// roundtrip with InvocationKind
impl std::fmt::Display for InvocationType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.kind())
    }
}

#[derive(Clone, Copy, Debug, derive_more::Display, derive_more::FromStr, Eq, PartialEq)]
#[display(rename_all = "lowercase")]
#[from_str(rename_all = "lowercase")]
pub enum InvocationKind {
    InPlace,
    Interactive,
    ShellCommand,
    ExecCommand,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invocation_type_display_round_trips_to_kind() {
        let cases = [
            (InvocationType::InPlace, InvocationKind::InPlace),
            (InvocationType::Interactive, InvocationKind::Interactive),
            (
                InvocationType::ShellCommand("echo hi".to_string()),
                InvocationKind::ShellCommand,
            ),
            (
                InvocationType::ExecCommand(vec!["ls".to_string(), "-l".to_string()]),
                InvocationKind::ExecCommand,
            ),
        ];

        for (invocation_type, kind) in cases {
            assert_eq!(invocation_type.kind(), kind);
            assert_eq!(
                invocation_type
                    .to_string()
                    .parse::<InvocationKind>()
                    .unwrap(),
                kind,
            );
        }
    }

    /// A context file written before `sandbox_mode` existed has no such
    /// field; it must still deserialize, defaulting the mode to `Off`.
    #[test]
    fn attach_ctx_without_sandbox_mode_deserializes_to_off() {
        let json = r#"{
            "env": "/flox_env",
            "env_cache": "/cache",
            "env_description": "myproject",
            "flox_active_environments": "[]",
            "prompt_color_1": "1",
            "prompt_color_2": "2",
            "flox_prompt_environments": "",
            "set_prompt": true,
            "flox_env_cuda_detection": "1",
            "interpreter_path": "/interpreter"
        }"#;

        let ctx: AttachCtx = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.sandbox_mode, SandboxMode::Off);
        // metrics_host is likewise absent in older context files.
        assert_eq!(ctx.metrics_host, None);
    }

    #[test]
    fn attach_ctx_round_trips_sandbox_mode() {
        let json = r#"{
            "env": "/flox_env",
            "env_cache": "/cache",
            "env_description": "myproject",
            "flox_active_environments": "[]",
            "prompt_color_1": "1",
            "prompt_color_2": "2",
            "flox_prompt_environments": "",
            "set_prompt": true,
            "flox_env_cuda_detection": "1",
            "interpreter_path": "/interpreter",
            "sandbox_mode": "ask"
        }"#;

        let ctx: AttachCtx = serde_json::from_str(json).unwrap();
        assert_eq!(ctx.sandbox_mode, SandboxMode::Ask);
    }
}
