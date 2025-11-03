use std::collections::HashMap;
use std::env::Vars;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};
use clap::Args;
use flox_core::activate_data::ActivateData;
use flox_core::activations::activations_json_path;
use flox_core::shell::Shell;
use flox_core::util::default_nix_env_vars;
use indoc::formatdoc;
use is_executable::IsExecutable;
use itertools::Itertools;
#[cfg(target_os = "linux")]
use libc::{PR_SET_CHILD_SUBREAPER, prctl, setsid};
use log::debug;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork, getpid, getppid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use time::{Duration, OffsetDateTime};

use super::StartOrAttachArgs;
use super::SetReadyArgs;
use super::attach::AttachArgs;
use super::fix_paths::{fix_manpath_var, fix_path_var};
use super::set_env_dirs::fix_env_dirs_var;
use crate::cli::attach::AttachExclusiveArgs;
use crate::executive::executive;
use crate::shell_gen::Shell as ShellGen;
use crate::shell_gen::bash::{BashStartupArgs, generate_bash_startup_commands};
use crate::shell_gen::capture::{EnvDiff, ExportEnvDiff};
use crate::shell_gen::fish::{FishStartupArgs, generate_fish_startup_commands};
use crate::shell_gen::tcsh::{TcshStartupArgs, generate_tcsh_startup_commands};
use crate::shell_gen::zsh::{ZshStartupArgs, generate_zsh_startup_script};
use crate::{debug_command_env, debug_set_var};

#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// Path to JSON file containing activation data
    #[arg(long)]
    pub activate_data: PathBuf,
}

pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";
/// This variable is used in tests to override what path to use for the socket.
pub const FLOX_SERVICES_SOCKET_OVERRIDE_VAR: &str = "_FLOX_SERVICES_SOCKET_OVERRIDE";

// TODO
pub const FLOX_RUNTIME_DIR_VAR: &str = "FLOX_RUNTIME_DIR";
pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";

// TODO
pub const RM: &str = "rm";

/// Build a HashMap of environment variables to set from ActivateData.
/// This is the common logic used by both the CLI activation and executive processes.
pub fn build_activation_env_vars(data: &ActivateData) -> HashMap<&'static str, String> {
    let mut exports = HashMap::from([
        (
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            data.flox_active_environments.clone(),
        ),
        (FLOX_ENV_LOG_DIR_VAR, data.flox_env_log_dir.clone()),
        ("FLOX_PROMPT_COLOR_1", data.prompt_color_1.clone()),
        ("FLOX_PROMPT_COLOR_2", data.prompt_color_2.clone()),
        (
            FLOX_PROMPT_ENVIRONMENTS_VAR,
            data.flox_prompt_environments.clone(),
        ),
        ("_FLOX_SET_PROMPT", data.set_prompt.to_string()),
        (
            "_FLOX_ACTIVATE_STORE_PATH",
            data.flox_activate_store_path.clone(),
        ),
        (FLOX_RUNTIME_DIR_VAR, data.flox_runtime_dir.clone()),
        (
            "_FLOX_ENV_CUDA_DETECTION",
            data.flox_env_cuda_detection.clone(),
        ),
        (
            FLOX_ACTIVATE_START_SERVICES_VAR,
            data.flox_activate_start_services.to_string(),
        ),
        (FLOX_SERVICES_SOCKET_VAR, data.flox_services_socket.clone()),
    ]);

    if let Some(services_to_start) = &data.flox_services_to_start {
        exports.insert(FLOX_SERVICES_TO_START_VAR, services_to_start.clone());
    }

    exports.extend(default_nix_env_vars());

    // Preserve _FLOX_SHELL_FORCE if set, so it survives for subshells
    if let Ok(shell_override) = std::env::var("_FLOX_SHELL_FORCE") {
        if !shell_override.is_empty() {
            exports.insert("_FLOX_SHELL_FORCE", shell_override);
        }
    }

    exports
}

/// Set activation environment variables directly in the current process.
/// This should be called after replay_env() to ensure Rust-managed variables
/// (like FLOX_PROMPT_ENVIRONMENTS) are set correctly, overriding any stale
/// values from the activation script's environment capture.
///
/// # Safety
/// This function uses unsafe operations to modify the process environment.
pub fn set_activation_env_vars_in_process(data: &ActivateData) {
    let env_vars = build_activation_env_vars(data);

    for (key, value) in env_vars {
        debug_set_var!(key, value);
    }
}

/// Set early activation environment variables that don't depend on the activation script output.
/// These need to be set before forking the Executive so they're inherited by all child processes.
///
/// This includes:
/// - FLOX_ENV, FLOX_ENV_CACHE, FLOX_ENV_PROJECT, FLOX_ENV_DESCRIPTION
/// - Default Nix environment variables
///
/// # Safety
/// This function uses unsafe operations to modify the process environment.
fn set_early_activation_env_in_process(data: &ActivateData) {
    // Set the documented exposed variables
    debug_set_var!("FLOX_ENV", &data.env);
    debug_set_var!("FLOX_ENV_CACHE", data.env_cache.to_string_lossy().to_string());
    debug_set_var!("FLOX_ENV_PROJECT", data.env_project.to_string_lossy().to_string());
    debug_set_var!("FLOX_ENV_DESCRIPTION", &data.env_description);

    // Set default Nix environment variables
    for (key, value) in default_nix_env_vars() {
        debug_set_var!(key, value);
    }
}

/// Builder for creating shell-specific startup args structs.
/// Consolidates the repeated pattern of building BashStartupArgs, FishStartupArgs, etc.
struct StartupArgsBuilder<'a> {
    verbosity: u8,
    data: &'a ActivateData,
    flox_sourcing_rc: bool,
    activate_tracer: String,
}

impl<'a> StartupArgsBuilder<'a> {
    fn new(
        verbosity: u8,
        data: &'a ActivateData,
        flox_sourcing_rc: bool,
        activate_tracer: String,
    ) -> Self {
        Self {
            verbosity,
            data,
            flox_sourcing_rc,
            activate_tracer,
        }
    }

    fn build_bash_args(&self) -> BashStartupArgs {
        BashStartupArgs {
            flox_activate_tracelevel: self.verbosity as i32,
            activate_d: self.data.interpreter_path.join("activate.d"),
            flox_env: self.data.env.clone(),
            flox_env_cache: Some(self.data.env_cache.to_string_lossy().to_string()),
            flox_env_project: Some(self.data.env_project.to_string_lossy().to_string()),
            flox_env_description: Some(self.data.env_description.clone()),
            is_in_place: self.data.in_place,
            flox_sourcing_rc: self.flox_sourcing_rc,
            flox_activations: (&self.data.path_to_self).into(),
            flox_activate_tracer: self.activate_tracer.clone(),
        }
    }

    fn build_fish_args(&self) -> FishStartupArgs {
        FishStartupArgs {
            flox_activate_tracelevel: self.verbosity as i32,
            activate_d: self.data.interpreter_path.join("activate.d"),
            flox_env: self.data.env.clone(),
            flox_env_cache: Some(self.data.env_cache.to_string_lossy().to_string()),
            flox_env_project: Some(self.data.env_project.to_string_lossy().to_string()),
            flox_env_description: Some(self.data.env_description.clone()),
            is_in_place: self.data.in_place,
            flox_sourcing_rc: self.flox_sourcing_rc,
            flox_activations: (&self.data.path_to_self).into(),
            flox_activate_tracer: self.activate_tracer.clone(),
        }
    }

    fn build_tcsh_args(&self) -> TcshStartupArgs {
        TcshStartupArgs {
            flox_activate_tracelevel: self.verbosity as i32,
            activate_d: self.data.interpreter_path.join("activate.d"),
            flox_env: self.data.env.clone(),
            flox_env_cache: Some(self.data.env_cache.to_string_lossy().to_string()),
            flox_env_project: Some(self.data.env_project.to_string_lossy().to_string()),
            flox_env_description: Some(self.data.env_description.clone()),
            is_in_place: self.data.in_place,
            flox_sourcing_rc: self.flox_sourcing_rc,
            flox_activations: (&self.data.path_to_self).into(),
            flox_activate_tracer: self.activate_tracer.clone(),
        }
    }

    fn build_zsh_args(&self) -> ZshStartupArgs {
        ZshStartupArgs {
            flox_activate_tracelevel: self.verbosity as i32,
            activate_d: self.data.interpreter_path.join("activate.d"),
            flox_env: self.data.env.clone(),
            flox_env_cache: Some(self.data.env_cache.to_string_lossy().to_string()),
            flox_env_project: Some(self.data.env_project.to_string_lossy().to_string()),
            flox_env_description: Some(self.data.env_description.clone()),
            is_in_place: self.data.in_place,
            flox_sourcing_rc: self.flox_sourcing_rc,
            flox_activations: (&self.data.path_to_self).into(),
            flox_activate_tracer: self.activate_tracer.clone(),
        }
    }
}

/// Determines how shell-specific environment variables should be set
#[derive(Debug, Clone, Copy)]
enum ShellEnvSetup {
    /// Set variables in the current process using unsafe set_var
    InProcess,
    /// Set variables on a Command object using .env()
    OnCommand,
    /// Return export statements as script text
    AsScript,
}

/// Apply shell-specific environment setup (HOME for tcsh, ZDOTDIR for zsh).
/// Returns export statements when in AsScript mode, empty vec otherwise.
fn apply_shell_specific_env(
    shell: &Shell,
    setup_mode: ShellEnvSetup,
    command: Option<&mut Command>,
    interpreter_path: &std::path::Path,
) -> Result<Vec<String>> {
    use std::path::Path;
    match shell {
        Shell::Tcsh(_) => {
            let home_dir = interpreter_path
                .join("activate.d")
                .join("tcsh_home");
            match setup_mode {
                ShellEnvSetup::InProcess => {
                    if let Ok(home) = std::env::var("HOME") {
                        debug_set_var!("FLOX_ORIG_HOME", home);
                    }
                    debug_set_var!("HOME", home_dir.to_string_lossy().to_string());
                    Ok(vec![])
                },
                ShellEnvSetup::OnCommand => {
                    if let Some(cmd) = command {
                        if let Ok(home) = std::env::var("HOME") {
                            debug_command_env!(cmd, "FLOX_ORIG_HOME", home);
                        }
                        debug_command_env!(cmd, "HOME", home_dir.to_string_lossy().to_string());
                    }
                    Ok(vec![])
                },
                ShellEnvSetup::AsScript => {
                    // For in_place mode, tcsh doesn't need exports returned
                    // because it relies on environment inheritance
                    Ok(vec![])
                }
            }
        },
        Shell::Zsh(_) => {
            let zdotdir = interpreter_path
                .join("activate.d")
                .join("zdotdir");
            match setup_mode {
                ShellEnvSetup::InProcess => {
                    if let Ok(orig_zdotdir) = std::env::var("ZDOTDIR") {
                        debug_set_var!("FLOX_ORIG_ZDOTDIR", orig_zdotdir);
                    }
                    debug_set_var!("ZDOTDIR", zdotdir.to_string_lossy().to_string());
                    Ok(vec![])
                },
                ShellEnvSetup::OnCommand => {
                    if let Some(cmd) = command {
                        if let Ok(orig_zdotdir) = std::env::var("ZDOTDIR") {
                            debug_command_env!(cmd, "FLOX_ORIG_ZDOTDIR", orig_zdotdir);
                        }
                        debug_command_env!(cmd, "ZDOTDIR", zdotdir.to_string_lossy().to_string());
                    }
                    Ok(vec![])
                },
                ShellEnvSetup::AsScript => {
                    let mut exports = vec![];
                    if let Ok(orig_zdotdir) = std::env::var("ZDOTDIR") {
                        exports.push(format!(
                            "export FLOX_ORIG_ZDOTDIR={}",
                            shell_escape::escape(orig_zdotdir.into())
                        ));
                    }
                    exports.push(format!(
                        r#"export ZDOTDIR="{}""#,
                        zdotdir.to_string_lossy()
                    ));
                    Ok(exports)
                }
            }
        },
        _ => Ok(vec![])
    }
}

/// Result of script generation containing the script path and content
struct ScriptGeneration {
    script_path: PathBuf,
    script_content: String,
}

/// Generate activation script for the given shell.
/// Consolidates the pattern of: generate commands -> write to file -> return path
fn generate_activation_script(
    shell: &Shell,
    args_builder: &StartupArgsBuilder,
    export_env_diff: &ExportEnvDiff,
    activation_state_dir: &PathBuf,
    self_destruct: bool,
) -> Result<ScriptGeneration> {
    match shell {
        Shell::Bash(_) => {
            let args = args_builder.build_bash_args();
            let commands = generate_bash_startup_commands(&args, export_env_diff)?;
            let path = write_maybe_self_destructing_script(
                commands.clone(),
                activation_state_dir,
                self_destruct,
            )?;
            Ok(ScriptGeneration {
                script_path: path,
                script_content: commands,
            })
        },
        Shell::Fish(_) => {
            let args = args_builder.build_fish_args();
            let commands = generate_fish_startup_commands(&args, export_env_diff)?;
            let path = write_maybe_self_destructing_script(
                commands.clone(),
                activation_state_dir,
                self_destruct,
            )?;
            Ok(ScriptGeneration {
                script_path: path,
                script_content: commands,
            })
        },
        Shell::Tcsh(_) => {
            let args = args_builder.build_tcsh_args();
            let commands = generate_tcsh_startup_commands(&args, export_env_diff)?;
            let path = write_maybe_self_destructing_script(
                commands.clone(),
                activation_state_dir,
                self_destruct,
            )?;
            Ok(ScriptGeneration {
                script_path: path,
                script_content: commands,
            })
        },
        Shell::Zsh(_) => {
            let args = args_builder.build_zsh_args();
            let script = generate_zsh_startup_script(&args, export_env_diff)?;
            let path = write_maybe_self_destructing_script(
                script.clone(),
                activation_state_dir,
                self_destruct,
            )?;
            Ok(ScriptGeneration {
                script_path: path,
                script_content: script,
            })
        },
    }
}

/// Write script to a temporary file, optionally adding self-destruct command
fn write_maybe_self_destructing_script(
    mut script: String,
    activation_state_dir: &PathBuf,
    self_destruct: bool,
) -> Result<PathBuf> {
    let mut tempfile = tempfile::NamedTempFile::new_in(activation_state_dir)?;
    if self_destruct {
        script.push_str(&format!(
            "\ntrue {RM} {}",
            tempfile.path().to_string_lossy()
        ));
    }
    tempfile.write_all(script.as_bytes())?;
    let (_, path) = tempfile.keep()?;
    Ok(path)
}

impl ActivateArgs {
    pub fn handle(self, verbosity: u8) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let mut data: ActivateData = serde_json::from_str(&contents)?;

        fs::remove_file(&self.activate_data)?;

        // Check for _FLOX_SHELL_FORCE and use it to override data.shell if set
        // This allows users to specify the shell for activation and subshells
        if let Ok(shell_override) = std::env::var("_FLOX_SHELL_FORCE") {
            if !shell_override.is_empty() {
                debug!("Overriding shell from _FLOX_SHELL_FORCE: {}", shell_override);
                let shell_path = std::path::Path::new(&shell_override);
                match Shell::try_from(shell_path) {
                    Ok(shell) => {
                        data.shell = shell;
                    },
                    Err(e) => {
                        debug!("Failed to parse _FLOX_SHELL_FORCE value '{}': {}", shell_override, e);
                    }
                }
            }
        }

        // Set FLOX_ENV_DIRS and fix PATH/MANPATH before forking the executive
        // These were previously done by the bash activate script, but now we do them
        // in Rust before forking so that all processes inherit the correct environment
        crate::cli::set_env_dirs::set_env_dirs_in_process(&data.env)?;
        crate::cli::fix_paths::fix_paths_in_process()?;

        // Set additional activation environment variables before forking the executive
        // This ensures FLOX_ENV, FLOX_ENV_CACHE, FLOX_ENV_PROJECT, etc. are inherited
        // by the executive process and the activation script it executes
        set_early_activation_env_in_process(&data);

        let (attach, activation_state_dir, activation_id) = StartOrAttachArgs {
            pid: std::process::id() as i32,
            flox_env: PathBuf::from(&data.env),
            store_path: data.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&data.flox_runtime_dir),
        }
        .handle()?;

        if attach {
            eprintln!(
                "✅ Attached to existing activation of environment '{}'",
                data.env_description
            );
            eprintln!("To stop using this environment, type 'exit'");
            debug!(
                "Attaching to existing activation in state dir {:?}, id {}",
                activation_state_dir, activation_id
            );
        } else {
            let parent_pid = getpid();
            match unsafe { fork() } {
                Ok(ForkResult::Child) => {
                    unsafe {
                        setsid();
                        // register as subreaper on Linux only
                        #[cfg(target_os = "linux")]
                        prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0);
                    }
                    // Executive runs monitoring loop and exits when all PIDs are gone
                    if let Err(e) = executive(
                        data.clone(),
                        parent_pid,
                        activation_state_dir.clone(),
                        activation_id.clone(),
                    ) {
                        eprintln!("Executive failed: {}", e);
                        std::process::exit(1);
                    }
                    // Executive completed successfully - exit cleanly
                    std::process::exit(0);
                },
                Ok(ForkResult::Parent { child }) => {
                    // Parent process
                    debug!("Awaiting SIGUSR1 from child process with PID: {}", child);

                    // Set up signal handler to await the death of the child.
                    // If the child dies, then we should error out. We expect
                    // to receive SIGUSR1 from the child when it's ready.
                    let mut signals = Signals::new(&[SIGCHLD, SIGUSR1])?;
                    for signal in signals.forever() {
                        match signal {
                            SIGUSR1 => {
                                debug!("Received SIGUSR1 from child process {}", child);
                                break; // Proceed after receiving SIGUSR1
                            },
                            SIGCHLD => {
                                // SIGCHLD can come from any child process, not just ours.
                                // Use waitpid with WNOHANG to check if OUR child has exited.
                                match waitpid(child, Some(WaitPidFlag::WNOHANG)) {
                                    Ok(WaitStatus::StillAlive) => {
                                        // Our child is still alive, SIGCHLD was from a different process
                                        debug!(
                                            "Received SIGCHLD but child {} is still alive, continuing to wait",
                                            child
                                        );
                                        continue;
                                    },
                                    Ok(status) => {
                                        // Our child has exited
                                        debug!(
                                            "Child process {} exited unexpectedly with status: {:?}",
                                            child, status
                                        );
                                        return Err(anyhow!(
                                            "Activation process {} terminated unexpectedly with status: {:?}",
                                            child,
                                            status
                                        ));
                                    },
                                    Err(nix::errno::Errno::ECHILD) => {
                                        // Child already reaped, this shouldn't happen but handle gracefully
                                        debug!(
                                            "Received SIGCHLD but child {} already reaped",
                                            child
                                        );
                                        return Err(anyhow!(
                                            "Activation process {} terminated unexpectedly (already reaped)",
                                            child
                                        ));
                                    },
                                    Err(e) => {
                                        // Unexpected error from waitpid
                                        return Err(anyhow!(
                                            "Failed to check status of activation process {}: {}",
                                            child,
                                            e
                                        ));
                                    },
                                }
                            },
                            _ => unreachable!(),
                        }
                    }

                    // Mark the activation as ready now that we've received SIGUSR1 from the executive
                    debug!("Marking activation {} as ready", activation_id);
                    SetReadyArgs {
                        flox_env: PathBuf::from(&data.env),
                        id: activation_id.clone(),
                        runtime_dir: PathBuf::from(&data.flox_runtime_dir),
                    }
                    .handle()
                    .context("Failed to mark activation as ready")?;
                },
                Err(e) => {
                    // Fork failed
                    return Err(anyhow!("Fork failed: {}", e));
                },
            }
            debug!("Finished spawning activation - proceeding to attach");
        }

        let mut command = Self::assemble_command_for_activate_script(data.clone());

        // Replay environment variables directly in the Rust process
        // This implements the replayEnv() step from the Mermaid diagram
        crate::shell_gen::capture::replay_env(
            activation_state_dir.join("start.env.json"),
            activation_state_dir.join("end.env.json"),
        )?;

        // After replaying the environment from the activation script, explicitly set
        // the Rust-managed variables (like FLOX_PROMPT_ENVIRONMENTS) to their correct values.
        // This ensures they override any stale values captured by the activation script.
        set_activation_env_vars_in_process(&data);

        let export_env_diff = ExportEnvDiff::from_files(
            activation_state_dir.join("start.env.json"),
            activation_state_dir.join("end.env.json"),
        )?;
        let env_diff: EnvDiff = (&export_env_diff).try_into()?;
        let vars_from_environment = VarsFromEnvironment::get()?;
        let activation_environment = Self::assemble_environment(
            data.clone(),
            vars_from_environment,
            env_diff,
            &activation_state_dir,
            &activation_id,
        )?;

        // Convert activation_environment (which includes FLOX_ENV_CACHE, FLOX_ENV_PROJECT, etc.)
        // to ExportEnvDiff for use by activation functions
        let activation_export_env_diff: ExportEnvDiff = activation_environment.into();

        if attach {
            // TODO: print message about attaching
        }
        if data.flox_activate_start_services {
            Self::start_services();
        }

        // The activate_tracer is set from the FLOX_ACTIVATE_TRACE env var.
        // If that env var is empty then activate_tracer is set to the full path of the `true` command in the PATH.
        // If that env var is not empty and refers to an executable then then activate_tracer is set to that value.
        // Else activate_tracer is set to refer to {data.interpreter_path}/activate.d/trace.
        let activate_tracer = if let Ok(trace_path) = std::env::var("FLOX_ACTIVATE_TRACE") {
            if !trace_path.is_empty() && std::path::Path::new(&trace_path).is_executable() {
                trace_path
            } else {
                data.interpreter_path
                    .join("activate.d")
                    .join("trace")
                    .to_string_lossy()
                    .to_string()
            }
        } else {
            "true".to_string()
        };

        if data.in_place {
            let flox_sourcing_rc = std::env::var("_flox_sourcing_rc")
                .map(|v| v == "true")
                .unwrap_or(false);
            let legacy_exports = Self::render_legacy_exports(&command, &data.shell);
            Self::new_activate_in_place(
                data,
                activation_id,
                activation_state_dir,
                legacy_exports,
                flox_sourcing_rc,
                verbosity,
                activation_export_env_diff,
                activate_tracer,
            )?;
            return Ok(());
        }

        // These functions will only return if exec fails or for an
        // ephemeral activation
        if data.interactive {
            Self::new_activate_interactive(
                verbosity,
                data,
                std::env::var("_flox_sourcing_rc")
                    .map(|v| v == "true")
                    .unwrap_or(false),
                activation_export_env_diff,
                &activation_state_dir,
                activate_tracer,
            )?;
        } else if let Some(ref command_string) = data.command_string {
            Self::new_activate_command_string(
                verbosity,
                data,
                std::env::var("_flox_sourcing_rc")
                    .map(|v| v == "true")
                    .unwrap_or(false),
                activation_export_env_diff,
                &activation_state_dir,
                activate_tracer,
            )?;
        } else {
            Self::new_activate_command(
                data.run_args,
                data.is_ephemeral,
                activation_export_env_diff,
            )?;
        }

        return Ok(());
    }

    fn assemble_command_for_activate_script(data: ActivateData) -> Command {
        let exports = build_activation_env_vars(&data);

        let activate_path = data.interpreter_path.join("activate");
        let mut command = Command::new(activate_path);
        command.envs(exports);

        command.arg("--env").arg(&data.env);

        command.arg("--shell").arg(data.shell.exe_path());

        command
    }

    fn assemble_command_with_environment(
        bin: impl AsRef<OsStr>,
        activate_environment: &EnvDiff,
    ) -> Command {
        let mut command = Command::new(bin);

        command.envs(&activate_environment.additions);
        for key in &activate_environment.deletions {
            command.env_remove(key);
        }

        command
    }

    fn assemble_environment(
        data: ActivateData,
        vars_from_environment: VarsFromEnvironment,
        mut env_diff: EnvDiff,
        activation_state_dir: &PathBuf,
        activation_id: &str,
    ) -> Result<EnvDiff> {
        let mut additions_static_str = HashMap::new();

        additions_static_str.extend(Self::assemble_fixed_vars(&data.env, vars_from_environment));

        // TODO: dedup with shell_gen specific code
        // Propagate required variables that are documented as exposed.
        additions_static_str.insert("FLOX_ENV", data.env);
        // Propagate optional variables that are documented as exposed.
        additions_static_str.insert(
            "FLOX_ENV_CACHE",
            data.env_cache.to_string_lossy().to_string(),
        );

        additions_static_str.insert(
            "FLOX_ENV_PROJECT",
            data.env_project.to_string_lossy().to_string(),
        );

        additions_static_str.insert("FLOX_ENV_DESCRIPTION", data.env_description);

        // Propagate activation-specific variables
        additions_static_str.insert(
            "FLOX_ACTIVATION_STATE_DIR",
            activation_state_dir.to_string_lossy().to_string(),
        );
        additions_static_str.insert("FLOX_ACTIVATION_ID", activation_id.to_string());

        // Do we need this or will this already be inherited?
        additions_static_str.extend(default_nix_env_vars());

        // Preserve _FLOX_SHELL_FORCE if set, so it survives for subshells
        if let Ok(shell_override) = std::env::var("_FLOX_SHELL_FORCE") {
            if !shell_override.is_empty() {
                additions_static_str.insert("_FLOX_SHELL_FORCE", shell_override);
            }
        }

        env_diff.additions.extend(
            additions_static_str
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        );

        // Always unset the FLOX_SHELL variable as we attach to an activation.
        env_diff.deletions.extend(vec!["FLOX_SHELL".to_string()]);

        debug!("Final assembled environment diff additions: {:?}", env_diff.additions);
        debug!("Final assembled environment diff deletions: {:?}", env_diff.deletions);

        Ok(env_diff)
    }

    fn assemble_fixed_vars(
        flox_env: impl AsRef<str>,
        vars_from_environment: VarsFromEnvironment,
    ) -> HashMap<&'static str, String> {
        let new_flox_env_dirs = fix_env_dirs_var(
            flox_env.as_ref(),
            vars_from_environment
                .flox_env_dirs
                .unwrap_or("".to_string()),
        );
        let new_path = fix_path_var(&new_flox_env_dirs, &vars_from_environment.path);
        let new_manpath = fix_manpath_var(
            &new_flox_env_dirs,
            &vars_from_environment.manpath.unwrap_or("".to_string()),
        );
        HashMap::from([
            ("FLOX_ENV_DIRS", new_flox_env_dirs),
            ("PATH", new_path),
            ("MANPATH", new_manpath),
        ])
    }

    /// Used for `flox activate -- run_args`
    fn activate_command(
        mut command: Command,
        run_args: Vec<String>,
        is_ephemeral: bool,
    ) -> Result<()> {
        // The activation script works like a shell in that it accepts the "-c"
        // flag which takes exactly one argument to be passed verbatim to the
        // userShell invocation. Take this opportunity to combine these args
        // safely, and *exactly* as the user provided them in argv.
        command.arg("-c").arg(Self::quote_run_args(&run_args));

        debug!("running activation command: {:?}", command);

        if is_ephemeral {
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                Err(anyhow!(
                    "failed to run activation script: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))?;
            }
            Ok(())
        } else {
            // exec should never return
            Err(command.exec().into())
        }
    }

    /// Execute the user's command directly using exec().
    ///
    /// The environment modifications from activation are applied to the command
    /// before execution.
    fn new_activate_command(
        run_args: Vec<String>,
        is_ephemeral: bool,
        export_env_diff: ExportEnvDiff,
    ) -> Result<()> {
        if run_args.is_empty() {
            return Err(anyhow!("empty command provided"));
        }
        let user_command = &run_args[0];
        let args = &run_args[1..];

        // Convert export_env_diff to EnvDiff and apply it to the command
        let env_diff: EnvDiff = (&export_env_diff).try_into()?;
        let mut command = Self::assemble_command_with_environment(user_command, &env_diff);
        command.args(args);

        debug!("executing command directly: {:?}", command);

        if is_ephemeral {
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                Err(anyhow!(
                    "failed to run command: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))?;
            }
            Ok(())
        } else {
            // exec replaces the current process - should never return
            Err(command.exec().into())
        }
    }

    /// Used for `flox activate -c "command string"`
    fn new_activate_command_string(
        verbosity: u8,
        data: ActivateData,
        flox_sourcing_rc: bool,
        export_env_diff: ExportEnvDiff,
        activation_state_dir: &PathBuf,
        activate_tracer: String,
    ) -> Result<()> {
        // Set _flox_* variables in the environment prior to sourcing
        // our custom .tcshrc and ZDOTDIR that will otherwise fall over.
        /// SAFETY: called once, prior to possible concurrent access to env
        debug_set_var!(
            "_activate_d",
            data.interpreter_path.join("activate.d").to_string_lossy().to_string()
        );
        debug_set_var!("_flox_activate_tracelevel", verbosity.to_string());

        // Build the startup args once for all shells
        let args_builder = StartupArgsBuilder::new(
            verbosity,
            &data,
            flox_sourcing_rc,
            activate_tracer,
        );

        // Generate the activation script
        let script_gen = generate_activation_script(
            &data.shell,
            &args_builder,
            &export_env_diff,
            activation_state_dir,
            verbosity < 2,
        )?;

        let env_diff: EnvDiff = (&export_env_diff).try_into()?;
        let mut command = Self::assemble_command_with_environment(data.shell.exe_path(), &env_diff);

        // Apply shell-specific environment setup
        apply_shell_specific_env(
            &data.shell,
            ShellEnvSetup::OnCommand,
            Some(&mut command),
            &data.interpreter_path,
        )?;

        // Configure shell-specific command arguments
        match data.shell {
            Shell::Bash(_) => {
                debug_command_env!(&mut command, "FLOX_BASH_INIT_SCRIPT", script_gen.script_path.to_string_lossy().to_string());
                command.args(["--noprofile", "--rcfile", &script_gen.script_path.to_string_lossy()]);
                // Invoke bash -c "source $FLOX_BASH_INIT_SCRIPT; <command string>".
                command.arg("-c").arg(formatdoc!(
                    r#"
                    source '{}';
                    {};
                    "#,
                    script_gen.script_path.to_string_lossy(),
                    data.command_string.unwrap()
                ));
            },
            Shell::Fish(_) => {
                // Not strictly required, but good to have in the env for debugging
                // and for parity with the tcsh/zsh shells that need it.
                debug_command_env!(&mut command, "FLOX_FISH_INIT_SCRIPT", script_gen.script_path.to_string_lossy().to_string());
                command.args([
                    "--init-command",
                    format!("source '{}'", &script_gen.script_path.to_string_lossy()).as_str(),
                ]);
                command.arg("-c").arg(data.command_string.unwrap());
            },
            Shell::Tcsh(_) => {
                debug_command_env!(&mut command, "FLOX_TCSH_INIT_SCRIPT", script_gen.script_path.to_string_lossy().to_string());
                command.arg("-c").arg(data.command_string.unwrap());
            },
            Shell::Zsh(_) => {
                // export FLOX_ZSH_INIT_SCRIPT so that it can be sourced from ZDOTDIR.
                debug_command_env!(&mut command, "FLOX_ZSH_INIT_SCRIPT",
                    data.interpreter_path.join("activate.d").join("zsh").to_string_lossy().to_string());
                    // script_gen.script_path.to_string_lossy().to_string());
                command.arg("-c").arg(data.command_string.unwrap());
            },
            _ => unimplemented!(),
        }

        debug!("running command string in shell: {:?}", command);
        if data.is_ephemeral {
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                Err(anyhow!(
                    "failed to run command string: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))?;
            }
            Ok(())
        } else {
            // exec should never return
            Err(command.exec().into())
        }
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn activate_interactive(mut command: Command) -> Result<()> {
        debug!("running activation command: {:?}", command);

        // exec should never return
        Err(command.exec().into())
    }

    fn new_activate_interactive(
        verbosity: u8,
        data: ActivateData,
        flox_sourcing_rc: bool,
        export_env_diff: ExportEnvDiff,
        activation_state_dir: &PathBuf,
        activate_tracer: String,
    ) -> Result<()> {
        // Print greeting message to STDERR.
        if data.interactive && !data.env_description.is_empty() {
            eprintln!(
                "✅ You are now using the environment '{}'.",
                data.env_description
            );
            eprintln!("To stop using this environment, type 'exit'");
            eprintln!();
        }

        // Set _flox_* variables in the environment prior to sourcing
        // our custom .tcshrc and ZDOTDIR that will otherwise fall over.
        /// SAFETY: called once, prior to possible concurrent access to env
        debug_set_var!(
            "_activate_d",
            data.interpreter_path.join("activate.d").to_string_lossy().to_string()
        );
        debug_set_var!("_flox_activate_tracelevel", verbosity.to_string());

        // Build the startup args once for all shells
        let args_builder = StartupArgsBuilder::new(
            verbosity,
            &data,
            flox_sourcing_rc,
            activate_tracer,
        );

        // Generate the activation script
        let script_gen = generate_activation_script(
            &data.shell,
            &args_builder,
            &export_env_diff,
            activation_state_dir,
            verbosity < 2,
        )?;

        // Apply shell-specific environment setup (InProcess mode for interactive)
        apply_shell_specific_env(
            &data.shell,
            ShellEnvSetup::InProcess,
            None,
            &data.interpreter_path,
        )?;

        // Set shell-specific init script environment variables
        match &data.shell {
            Shell::Tcsh(_) => {
                /// SAFETY: called once, prior to possible concurrent access to env
                debug_set_var!(
                    "FLOX_TCSH_INIT_SCRIPT",
                    script_gen.script_path.to_string_lossy().to_string()
                );
            },
            Shell::Zsh(_) => {
                /// SAFETY: called once, prior to possible concurrent access to env
                debug_set_var!(
                    "FLOX_ZSH_INIT_SCRIPT",
                    // script_gen.script_path.to_string_lossy().to_string()
                    data.interpreter_path.join("activate.d").join("zsh").to_string_lossy().to_string()
                );
            },
            _ => {}
        }

        // Create and configure the command for each shell
        match data.shell {
            Shell::Bash(bash) => {
                let mut command = Command::new(bash);
                command.args(["--noprofile", "--rcfile", &script_gen.script_path.to_string_lossy()]);

                debug!("spawning interactive bash shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
                // TODO: do we need to port this case?
                // # The bash --rcfile option only works for interactive shells
                // # so we need to cobble together our own means of sourcing our
                // # startup script for non-interactive shells.
                // # XXX Is this case even a thing? What's the point of activating with
                // #     no command to be invoked and no controlling terminal from which
                // #     to issue commands?!? A broken docker experience maybe?!?
                // exec "$_flox_shell" --noprofile --norc -s <<< "source '$RCFILE'"
            },
            Shell::Fish(fish) => {
                let mut command = Command::new(fish);
                command.args([
                    "--init-command",
                    format!("source '{}'", &script_gen.script_path.to_string_lossy()).as_str(),
                ]);

                debug!("spawning interactive fish shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            Shell::Tcsh(tcsh) => {
                let mut command = Command::new(tcsh);
                command.args(["-m"]);

                debug!("spawning interactive tcsh shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            Shell::Zsh(zsh) => {
                let mut command = Command::new(zsh);
                command.args(["-o", "NO_GLOBAL_RCS"]);

                debug!("spawning interactive zsh shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            _ => unimplemented!(),
        }

    }

    fn render_legacy_exports(command: &Command, shell: &Shell) -> String {
        // Render the exports in the correct shell dialect.
        command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|v| {
                    (
                        key.to_string_lossy(),
                        shell_escape::escape(v.to_string_lossy()),
                    )
                })
            })
            .map(|(key, value)| match shell {
                Shell::Bash(_) => format!("export {key}={value};",),
                Shell::Fish(_) => format!("set -gx {key} {value};",),
                Shell::Tcsh(_) => format!("setenv {key} {value};",),
                Shell::Zsh(_) => format!("export {key}={value};",),
            })
            .join("\n")
    }

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(mut command: Command, shell: Shell) -> Result<()> {
        debug!("activating in place with command: {:?}", command);

        let output = command
            .output()
            .context("failed to run activation script")?;
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        let exports_rendered = Self::render_legacy_exports(&command, &shell);

        let script = formatdoc! {"
            {exports_rendered}
            {output}
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");

        Ok(())
    }

    fn new_activate_in_place(
        data: ActivateData,
        activation_id: String,
        activation_state_dir: PathBuf,
        legacy_exports: String,
        flox_sourcing_rc: bool,
        verbosity: u8,
        export_env_diff: ExportEnvDiff,
        activate_tracer: String,
    ) -> Result<()> {
        let attach_command = AttachArgs {
            pid: std::process::id() as i32,
            flox_env: (&data.env).into(),
            id: activation_id.clone(),
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(5000),
                remove_pid: None,
            },
            runtime_dir: (&data.flox_runtime_dir).into(),
        };

        // Put a 5 second timeout on the activation
        attach_command.handle()?;

        // Build the startup args once for all shells
        let args_builder = StartupArgsBuilder::new(
            verbosity,
            &data,
            flox_sourcing_rc,
            activate_tracer.clone(),
        );

        let startup_commands = match data.shell {
            Shell::Bash(_) => {
                // For bash/fish/tcsh, we don't write a script file - just generate inline commands
                let args = args_builder.build_bash_args();
                let startup_commands = generate_bash_startup_commands(&args, &export_env_diff)?;

                formatdoc! {r#"
                  {flox_activations} attach --runtime-dir "{runtime_dir}" --pid $$ --flox-env "{flox_env}" --id "{id}" --remove-pid "{pid}";
                  {startup_commands}
                "#,
                // TODO: this should probably be based on interpreter_path
                flox_activations = data.path_to_self,
                runtime_dir = data.flox_runtime_dir,
                flox_env = data.env,
                id = activation_id,
                pid = std::process::id() }
            },
            Shell::Fish(_) => {
                let args = args_builder.build_fish_args();
                let startup_commands = generate_fish_startup_commands(&args, &export_env_diff)?;

                formatdoc! {r#"
                  {flox_activations} attach --runtime-dir "{runtime_dir}" --pid $fish_pid --flox-env "{flox_env}" --id "{id}" --remove-pid "{pid}";
                  {startup_commands}
                "#,
                // TODO: this should probably be based on interpreter_path
                flox_activations = data.path_to_self,
                runtime_dir = data.flox_runtime_dir,
                flox_env = data.env,
                id = activation_id,
                pid = std::process::id() }
            },
            Shell::Tcsh(_) => {
                let args = args_builder.build_tcsh_args();
                let startup_commands = generate_tcsh_startup_commands(&args, &export_env_diff)?;

                formatdoc! {r#"
                  {flox_activations} attach --runtime-dir "{runtime_dir}" --pid $$ --flox-env "{flox_env}" --id "{id}" --remove-pid "{pid}";
                  {startup_commands}
                "#,
                // TODO: this should probably be based on interpreter_path
                flox_activations = data.path_to_self,
                runtime_dir = data.flox_runtime_dir,
                flox_env = data.env,
                id = activation_id,
                pid = std::process::id() }
            },
            Shell::Zsh(_) => {
                // Zsh is special: it needs to write a script file and build commands manually
                let script_gen = generate_activation_script(
                    &data.shell,
                    &args_builder,
                    &export_env_diff,
                    &activation_state_dir,
                    false, // Don't self-destruct for in-place mode
                )?;

                // Get shell-specific environment exports as script text
                let env_exports = apply_shell_specific_env(
                    &data.shell,
                    ShellEnvSetup::AsScript,
                    None,
                    &data.interpreter_path,
                )?;

                let mut commands = Vec::new();

                commands.push(format!(
                    r#"{} attach --runtime-dir "{}" --pid $$ --flox-env "{}" --id "{}" --remove-pid {}"#,
                    // TODO: this should probably be based on interpreter_path
                    data.path_to_self,
                    data.flox_runtime_dir,
                    data.env,
                    activation_id,
                    std::process::id(),
                ));

                // Add the environment-specific exports (ZDOTDIR, etc.)
                commands.extend(env_exports);

                // Export the value of $_flox_activate_tracer from the environment.
                commands.push(ShellGen::Zsh.export_var(
                    "_flox_activate_tracer",
                    &activate_tracer,
                ));

                commands.push(format!(
                    "source '{}'",
                    script_gen.script_path.to_string_lossy(),
                ));

                // N.B. the output of these scripts may be eval'd with backticks which have
                // the effect of removing newlines from the output, so we must ensure that
                // the output is a valid shell script fragment when represented on a single line.
                commands.push("".to_string()); // ensure there's a trailing newline
                commands.join(";\n")
            },
            _ => unimplemented!(),
        };
        let script = formatdoc! {"
            {legacy_exports}
            {startup_commands}
        "};
        print!("{script}");
        Ok(())
    }

    /// Quote run args so that words don't get split,
    /// but don't escape all characters.
    ///
    /// To do this we escape '"' and '`',
    /// but we don't escape anything else.
    /// We want '$' for example to be expanded by the shell.
    fn quote_run_args(run_args: &[String]) -> String {
        run_args
            .iter()
            .map(|arg| {
                if arg.contains(' ') || arg.contains('"') || arg.contains('`') {
                    format!(r#""{}""#, arg.replace('"', r#"\""#).replace('`', r#"\`"#))
                } else {
                    arg.to_string()
                }
            })
            .join(" ")
    }

    fn start_services() {
        // This function is called after attaching to an existing activation
        // It starts services via the process-compose socket if needed

        // Get the socket path from the environment
        let socket_path = match std::env::var(FLOX_SERVICES_SOCKET_VAR) {
            Ok(path) => path,
            Err(_) => {
                debug!("FLOX_SERVICES_SOCKET not set, skipping service start");
                return;
            },
        };

        // Get the services to start from the environment (JSON array)
        let services_json = match std::env::var(FLOX_SERVICES_TO_START_VAR) {
            Ok(json) => json,
            Err(_) => {
                debug!("No services specified to start");
                return;
            },
        };

        // Parse the JSON array of service names
        let services: Vec<String> = match serde_json::from_str(&services_json) {
            Ok(services) => services,
            Err(e) => {
                debug!("Failed to parse services JSON: {}", e);
                return;
            },
        };

        if services.is_empty() {
            debug!("No services to start");
            return;
        }

        // Start the services via the process-compose socket
        debug!("Starting services: {:?}", services);
        if let Err(e) = crate::process_compose::start_services(&socket_path, &services) {
            debug!("Failed to start services: {}", e);
            // Don't fail the activation - just log the error
        }
    }
}

struct VarsFromEnvironment {
    flox_env_dirs: Option<String>,
    path: String,
    manpath: Option<String>,
}

impl VarsFromEnvironment {
    fn get() -> Result<Self> {
        let flox_env_dirs = std::env::var("FLOX_ENV_DIRS").ok();
        let path = match std::env::var("PATH") {
            Ok(path) => path,
            Err(e) => {
                return Err(anyhow!("failed to get PATH from environment: {}", e));
            },
        };
        let manpath = std::env::var("MANPATH").ok();

        Ok(Self {
            flox_env_dirs,
            path,
            manpath,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_run_args() {
        assert_eq!(
            ActivateArgs::quote_run_args(&["a b".to_string(), '"'.to_string()]),
            r#""a b" "\"""#
        )
    }
}
