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
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

impl ActivateArgs {
    pub fn handle(self, verbosity: u8) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let data: ActivateData = serde_json::from_str(&contents)?;

        fs::remove_file(&self.activate_data)?;

        // Set FLOX_ENV_DIRS and fix PATH/MANPATH before forking the executive
        // These were previously done by the bash activate script, but now we do them
        // in Rust before forking so that all processes inherit the correct environment
        crate::cli::set_env_dirs::set_env_dirs_in_process(&data.env)?;
        crate::cli::fix_paths::fix_paths_in_process()?;

        let (attach, activation_state_dir, activation_id) = StartOrAttachArgs {
            pid: std::process::id() as i32,
            flox_env: PathBuf::from(&data.env),
            store_path: data.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&data.flox_runtime_dir),
        }
        .handle()?;

        if attach {
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
        let activation_environment =
            Self::assemble_environment(data.clone(), vars_from_environment, env_diff)?;
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
                export_env_diff,
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
                export_env_diff,
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
                export_env_diff,
                &activation_state_dir,
                activate_tracer,
            )?;
        } else {
            Self::new_activate_command(data.run_args, data.is_ephemeral)?;
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

        // Do we need this or will this already be inherited?
        additions_static_str.extend(default_nix_env_vars());

        env_diff.additions.extend(
            additions_static_str
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        );

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
    /// The environment has already been replayed via replay_env(), so the command
    /// will inherit the properly modified environment from the current process.
    fn new_activate_command(run_args: Vec<String>, is_ephemeral: bool) -> Result<()> {
        if run_args.is_empty() {
            return Err(anyhow!("empty command provided"));
        }
        let user_command = &run_args[0];
        let args = &run_args[1..];

        // Create command directly - it will inherit the already-replayed environment
        let mut command = Command::new(user_command);
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
        unsafe {
            std::env::set_var(
                "_activate_d", data.interpreter_path.join("activate.d")
              .to_string_lossy().to_string(),
            );
            std::env::set_var(
                "_flox_activate_tracelevel", verbosity.to_string(),
            );
        }

        let env_diff: EnvDiff = (&export_env_diff).try_into()?;
        let mut command = Self::assemble_command_with_environment(data.shell.exe_path(), &env_diff);

        match data.shell {
            Shell::Bash(bash) => {
                let bash_startup_args = BashStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_bash_startup_commands(&bash_startup_args, &export_env_diff)?;
                let rcfile = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;
                command.env("FLOX_BASH_INIT_SCRIPT", rcfile.to_string_lossy().to_string());
                command.args(["--noprofile", "--rcfile", &rcfile.to_string_lossy()]);
                // Invoke bash -c "source $FLOX_BASH_INIT_SCRIPT; <command string>".
                command.arg("-c").arg(formatdoc!(
                    r#"
                    source '{}';
                    {};
                    "#,
                    rcfile.to_string_lossy(),
                    data.command_string.unwrap()
                ));
            },
            Shell::Fish(fish) => {
                let fish_startup_args = FishStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_fish_startup_commands(&fish_startup_args, &export_env_diff)?;
                let rcfile = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;
                // Not strictly required, but good to have in the env for debugging
                // and for parity with the tcsh/zsh shells that need it.
                command.env("FLOX_FISH_INIT_SCRIPT", rcfile.to_string_lossy().to_string());

                command.args([
                    "--init-command",
                    format!("source '{}'", &rcfile.to_string_lossy()).as_str(),
                ]);
                command.arg("-c").arg(data.command_string.unwrap());
            },
            Shell::Tcsh(tcsh) => {
                let tcsh_startup_args = TcshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };

                // Capture original value of $HOME in $FLOX_ORIG_HOME so that it can be restored later.
                if let Ok(home) = std::env::var("HOME") {
                    command.env("FLOX_ORIG_HOME", home);
                }

                // Export HOME to point to activate.d/tcsh_home dir containing
                // our custom .tcshrc.
                command.env("HOME", tcsh_startup_args
                            .activate_d
                            .join("tcsh_home")
                            .to_string_lossy()
                            .to_string(),
                );

                let startup_commands =
                    generate_tcsh_startup_commands(&tcsh_startup_args, &export_env_diff)?;
                let flox_tcsh_init_script = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;

                command.env("FLOX_TCSH_INIT_SCRIPT", flox_tcsh_init_script.to_string_lossy().to_string());
                command.arg("-c").arg(data.command_string.unwrap());
            },
            Shell::Zsh(zsh) => {
                let zsh_startup_args = ZshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };

                // if the ZDOTDIR environment variable is set, export its value to the
                // environment as FLOX_ORIG_ZDOTDIR so that it can be restored later.
                if let Ok(zdotdir) = std::env::var("ZDOTDIR") {
                    command.env("FLOX_ORIG_ZDOTDIR", zdotdir);
                };

                // export ZDOTDIR to point to the activation state dir so that
                // .zshrc and .zshenv files are sourced from there.
                command.env("ZDOTDIR", zsh_startup_args
                    .activate_d
                    .join("zdotdir")
                    .to_string_lossy()
                    .to_string(),
                );

                let startup_script =
                    generate_zsh_startup_script(&zsh_startup_args, &export_env_diff)?;
                let flox_zsh_init_script = Self::write_maybe_self_destructing_script(
                    startup_script,
                    activation_state_dir,
                    verbosity < 2,
                )?;

                // export FLOX_ZSH_INIT_SCRIPT so that it can be sourced from ZDOTDIR.
                command.env("FLOX_ZSH_INIT_SCRIPT", flox_zsh_init_script.to_string_lossy().to_string());
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
        unsafe {
            std::env::set_var(
                "_activate_d", data.interpreter_path.join("activate.d")
              .to_string_lossy().to_string(),
            );
            std::env::set_var(
                "_flox_activate_tracelevel", verbosity.to_string(),
            );
        }

        match data.shell {
            Shell::Bash(bash) => {
                let bash_startup_args = BashStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_bash_startup_commands(&bash_startup_args, &export_env_diff)?;
                let rcfile = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;
                let mut command = Command::new(bash);
                command.args(["--noprofile", "--rcfile", &rcfile.to_string_lossy()]);

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
                let fish_startup_args = FishStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_fish_startup_commands(&fish_startup_args, &export_env_diff)?;
                let rcfile = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;
                let mut command = Command::new(fish);
                command.args([
                    "--init-command",
                    format!("source '{}'", &rcfile.to_string_lossy()).as_str(),
                ]);

                debug!("spawning interactive fish shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            Shell::Tcsh(tcsh) => {
                let tcsh_startup_args = TcshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };

                // Capture original value of $HOME in $FLOX_ORIG_HOME so that it can be restored later.
                /// SAFETY: called once, prior to possible concurrent access to env
                if let Ok(home) = std::env::var("HOME") {
                    unsafe {
                        std::env::set_var("FLOX_ORIG_HOME", home);
                    }
                }

                // Export HOME to point to activate.d/tcsh_home dir containing
                // our custom .tcshrc.
                /// SAFETY: called once, prior to possible concurrent access to env
                unsafe {
                    std::env::set_var(
                        "HOME",
                        tcsh_startup_args
                            .activate_d
                            .join("tcsh_home")
                            .to_string_lossy()
                            .to_string(),
                    );
                }

                let startup_commands =
                    generate_tcsh_startup_commands(&tcsh_startup_args, &export_env_diff)?;
                let flox_tcsh_init_script = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;

                /// SAFETY: called once, prior to possible concurrent access to env
                unsafe {
                    std::env::set_var(
                        "FLOX_TCSH_INIT_SCRIPT",
                        flox_tcsh_init_script.to_string_lossy().to_string(),
                    );
                }
                let mut command = Command::new(tcsh);
                command.args(["-m"]);

                debug!("spawning interactive tcsh shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            Shell::Zsh(zsh) => {
                let zsh_startup_args = ZshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };

                // if the ZDOTDIR environment variable is set, export its value to the
                // environment as FLOX_ORIG_ZDOTDIR so that it can be restored later.
                /// SAFETY: called once, prior to possible concurrent access to env
                if let Ok(zdotdir) = std::env::var("ZDOTDIR") {
                    unsafe {
                        std::env::set_var("FLOX_ORIG_ZDOTDIR", zdotdir);
                    }
                }

                // export ZDOTDIR to point to the activation state dir so that
                // .zshrc and .zshenv files are sourced from there.
                /// SAFETY: called once, prior to possible concurrent access to env
                unsafe {
                    std::env::set_var(
                        "ZDOTDIR",
                        zsh_startup_args
                            .activate_d
                            .join("zdotdir")
                            .to_string_lossy()
                            .to_string(),
                    );
                }

                let startup_script =
                    generate_zsh_startup_script(&zsh_startup_args, &export_env_diff)?;
                let flox_zsh_init_script = Self::write_maybe_self_destructing_script(
                    startup_script,
                    activation_state_dir,
                    verbosity < 2,
                )?;

                // export FLOX_ZSH_INIT_SCRIPT so that it can be sourced from ZDOTDIR.
                /// SAFETY: called once, prior to possible concurrent access to env
                unsafe {
                    std::env::set_var(
                        "FLOX_ZSH_INIT_SCRIPT",
                        flox_zsh_init_script.to_string_lossy().to_string(),
                    );
                }

                let mut command = Command::new(zsh);
                command.args(["-o", "NO_GLOBAL_RCS"]);

                debug!("spawning interactive zsh shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
            },
            _ => unimplemented!(),
        }

    }

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

        let startup_commands = match data.shell {
            Shell::Bash(_) => {
                let bash_startup_args = BashStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_bash_startup_commands(&bash_startup_args, &export_env_diff)?;

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
                let fish_startup_args = FishStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_fish_startup_commands(&fish_startup_args, &export_env_diff)?;

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
                let tcsh_startup_args = TcshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };
                let startup_commands =
                    generate_tcsh_startup_commands(&tcsh_startup_args, &export_env_diff)?;

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
                let zsh_startup_args = ZshStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                    flox_activate_tracer: activate_tracer,
                };

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

                // if the ZDOTDIR environment variable is set, add a command to export
                // the value of that variable to the environment so that it can be used
                // to restore the value of ZDOTDIR when the activation ends.
                if let Ok(zdotdir) = std::env::var("ZDOTDIR") {
                    commands.push(format!(
                        "export FLOX_ORIG_ZDOTDIR={}",
                        shell_escape::escape(zdotdir.into())
                    ));
                };

                commands.push(format!(
                    r#"export ZDOTDIR="{}""#,
                    data.interpreter_path
                        .join("activate.d")
                        .join("zdotdir")
                        .to_string_lossy()
                ));

                let startup_script =
                    generate_zsh_startup_script(&zsh_startup_args, &export_env_diff)?;
                let flox_zsh_init_script = Self::write_maybe_self_destructing_script(
                    startup_script,
                    &activation_state_dir,
                    false, // verbosity < 2,
                )?;

                // Export the value of $_flox_activate_tracer from the environment.
                commands.push(ShellGen::Zsh.export_var(
                    "_flox_activate_tracer",
                    &zsh_startup_args.flox_activate_tracer,
                ));

                commands.push(format!(
                    "source '{}'",
                    flox_zsh_init_script.to_string_lossy(),
                ));

                // N.B. the output of these scripts may be eval'd with backticks which have
                // the effect of removing newlines from the output, so we must ensure that
                // the output is a valid shell script fragment when represented on a single line.
                commands.push("".to_string()); // ensure there's a trailing newline
                let mut joined = commands.join(";\n");

                joined
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
