//! Start logic for activations.
//!
//! This module contains the core logic for starting new activations,
//! including spawning the executive process, running hooks, and
//! managing process-compose for services.

use std::collections::HashMap;
use std::fs::{self, DirBuilder};
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Result, anyhow, bail};
use indoc::indoc;
use flox_core::activate::context::{ActivateCtx, AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::{
    ActivationState,
    StartIdentifier,
    StartOrAttachResult,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use fslock::LockFile;
use nix::sys::signal::{Signal, kill};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{Pid, getpid};
use signal_hook::consts::{SIGCHLD, SIGUSR1};
use signal_hook::iterator::Signals;
use tracing::{debug, error, info_span, instrument};

use crate::activate_script_builder::old_cli_envs;
use crate::activate_script_builder::FLOX_ENV_DIRS_VAR;
use crate::cli::fix_paths::{fix_manpath_var, fix_path_var};
use crate::cli::set_env_dirs::fix_env_dirs_var;
use crate::cli::executive::ExecutiveCtx;
use crate::cli::setup_env::{
    ProfileEnvConfig, ToolPaths, compute_profile_env, parse_envrc,
};
// env diff file names (hardcoded, matching EnvDiff::from_files)
const ENV_DIFF_START_JSON: &str = "start.env.json";
const ENV_DIFF_END_JSON: &str = "end.env.json";
use crate::process_compose::{
    process_compose_down,
    start_services_via_socket,
    wait_for_socket_ready,
};
use crate::vars_from_env::VarsFromEnvironment;

/// Start a new activation because we either have a:
/// - different store path
/// - fresh state file, which could be caused by no executive
pub fn start(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: &VarsFromEnvironment,
    start_id: StartIdentifier,
    activations: &mut ActivationState,
    activations_json_path: &Path,
    lock: LockFile,
) -> Result<StartOrAttachResult, anyhow::Error> {
    let attach = &context.attach_ctx;
    let project = context
        .project_ctx
        .as_ref()
        .expect("start() requires project context");

    let start_state_dir = start_id.start_state_dir(&context.activation_state_dir)?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&start_state_dir)?;

    let new_executive = if !activations.executive_started() {
        // Register signal handler BEFORE spawning executive to avoid race condition
        // where SIGUSR1 arrives before handler is registered
        let signals = Signals::new([SIGCHLD, SIGUSR1])?;
        let exec_pid = spawn_executive(
            attach,
            project,
            &context.activation_state_dir,
            &start_state_dir,
        )?;
        activations.set_executive_pid(exec_pid.as_raw());
        Some((exec_pid, signals))
    } else {
        None
    };

    write_activations_json(activations, activations_json_path, lock)?;

    if let Some((exec_pid, signals)) = new_executive {
        wait_for_executive(exec_pid, signals)?;
    }

    run_activation(context, subsystem_verbosity, vars_from_env, &start_state_dir)?;

    // Re-acquire lock to mark ready
    let (activations_opt, lock) = read_activations_json(activations_json_path)?;
    let mut activations = activations_opt.expect("activations.json should exist");
    activations.set_ready(&start_id);
    write_activations_json(&activations, activations_json_path, lock)?;

    Ok(StartOrAttachResult::Start { start_id })
}

/// Start activation without executive (for containers).
/// Uses own PID as an "executive" to indicate container lifecycle.
pub fn start_without_executive(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: &VarsFromEnvironment,
    start_id: StartIdentifier,
    activations: &mut ActivationState,
    activations_json_path: &Path,
    lock: LockFile,
) -> Result<StartOrAttachResult, anyhow::Error> {
    let start_state_dir = start_id.start_state_dir(&context.activation_state_dir)?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&start_state_dir)?;

    let pid_self = std::process::id() as i32;
    activations.set_executive_pid(pid_self);
    write_activations_json(activations, activations_json_path, lock)?;

    run_activation(context, subsystem_verbosity, vars_from_env, &start_state_dir)?;

    // Mark ready
    let (activations_opt, lock) = read_activations_json(activations_json_path)?;
    let mut activations = activations_opt.expect("state.json should exist");
    activations.set_ready(&start_id);
    write_activations_json(&activations, activations_json_path, lock)?;

    Ok(StartOrAttachResult::Start { start_id })
}

/// Run the activation: compute profile env, parse envrc, and either skip bash
/// entirely (no hook) or run a minimal bash script (hook exists).
///
/// This replaces the previous approach of spawning a bash activate script that
/// would in turn spawn Rust subcommands. By computing everything in-process,
/// we save ~10-15ms of process spawn overhead.
#[instrument(name = "run_activation", skip_all)]
fn run_activation(
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: &VarsFromEnvironment,
    start_state_dir: &Path,
) -> Result<()> {
    let flox_env = &context.attach_ctx.env;
    let flox_env_path = Path::new(flox_env);
    let interpreter_path = &context.attach_ctx.interpreter_path;

    // 1. Compute activation context vars (FLOX_ENV_DIRS, PATH, MANPATH, etc.)
    //    These were previously set by assemble_activate_command on the bash Command.
    let mut activation_vars: HashMap<String, String> = HashMap::new();

    // old_cli_envs: FLOX_ACTIVE_ENVIRONMENTS, prompt colors, CUDA detection, etc.
    for (k, v) in old_cli_envs(&context.attach_ctx, context.project_ctx.as_ref()) {
        activation_vars.insert(k.to_string(), v);
    }

    // Core activation vars
    activation_vars.insert("FLOX_ENV".to_string(), context.attach_ctx.env.clone());
    activation_vars.insert(
        "FLOX_ENV_CACHE".to_string(),
        context.attach_ctx.env_cache.to_string_lossy().to_string(),
    );
    activation_vars.insert(
        "FLOX_ENV_DESCRIPTION".to_string(),
        context.attach_ctx.env_description.clone(),
    );
    if let Some(project) = &context.project_ctx {
        activation_vars.insert(
            "FLOX_ENV_PROJECT".to_string(),
            project.env_project.to_string_lossy().to_string(),
        );
    }

    // Compute updated FLOX_ENV_DIRS, PATH, MANPATH
    let new_env_dirs = fix_env_dirs_var(
        flox_env,
        vars_from_env.flox_env_dirs.as_deref().unwrap_or(""),
    );
    let new_path = fix_path_var(
        &new_env_dirs,
        &vars_from_env.path,
    );
    let new_manpath = fix_manpath_var(
        &new_env_dirs,
        &vars_from_env.manpath.as_deref().unwrap_or_default(),
    );
    activation_vars.insert(FLOX_ENV_DIRS_VAR.to_string(), new_env_dirs.clone());
    activation_vars.insert("PATH".to_string(), new_path);
    activation_vars.insert("MANPATH".to_string(), new_manpath);

    // 2. Capture start environment snapshot AFTER computing activation vars.
    //    In the old bash flow, start.env.json was captured inside bash which
    //    already had activation vars applied via Command::envs(). The env diff
    //    (start → end) must only contain profile.d + envrc changes, NOT
    //    activation context vars (those are applied separately during attach).
    let mut start_env: HashMap<String, String> = std::env::vars().collect();
    for (k, v) in &activation_vars {
        start_env.insert(k.clone(), v.clone());
    }
    let start_json_path = start_state_dir.join(ENV_DIFF_START_JSON);
    write_env_json(&start_json_path, &start_env)?;

    // 3. Compute profile.d env vars in-process (replaces setup-env subprocess)
    let tool_paths = ToolPaths::from_interpreter(interpreter_path)
        .unwrap_or_else(|_| ToolPaths::defaults());

    let config = ProfileEnvConfig {
        mode: context.mode.to_string(),
        flox_env: flox_env_path.to_path_buf(),
        env_dirs: new_env_dirs,
        ld_floxlib: tool_paths.ld_floxlib,
        ldconfig: tool_paths.ldconfig,
        find_bin: tool_paths.find,
        env_project: context.project_ctx.as_ref().map(|p| p.env_project.clone()),
    };

    let profile_env = compute_profile_env(&config)?;
    debug!(
        num_vars = profile_env.len(),
        "computed profile env vars in-process"
    );

    // 4. Parse envrc (manifest [vars] + SSL/locale defaults)
    let envrc_path = flox_env_path.join("activate.d/envrc");
    let envrc_vars = parse_envrc(&envrc_path)?;
    debug!(num_vars = envrc_vars.len(), "parsed envrc vars");

    // 5. Check if hook-on-activate exists
    let hook_path = flox_env_path.join("activate.d/hook-on-activate");
    let has_hook = hook_path.exists();

    if has_hook {
        debug!("hook-on-activate exists, running via bash");
        run_activation_with_hook(
            context,
            start_state_dir,
            &activation_vars,
            &profile_env,
            &envrc_vars,
            &hook_path,
        )?;
    } else {
        // No hook: compute end environment entirely in Rust.
        // end env = start env + activation context + profile vars + envrc vars
        debug!("no hook-on-activate, skipping bash entirely");
        let mut end_env = start_env;
        for (k, v) in &activation_vars {
            end_env.insert(k.clone(), v.clone());
        }
        for (k, v) in &profile_env {
            end_env.insert(k.clone(), v.clone());
        }
        for (k, v) in &envrc_vars {
            end_env.insert(k.clone(), v.clone());
        }
        let end_json_path = start_state_dir.join(ENV_DIFF_END_JSON);
        write_env_json(&end_json_path, &end_env)?;
    }

    Ok(())
}

/// Run activation with a hook-on-activate script via a minimal bash process.
/// All env vars (activation context, profile.d, envrc) are pre-applied via
/// Command::envs(), so bash only needs to source the hook and dump the
/// final environment.
fn run_activation_with_hook(
    context: &ActivateCtx,
    start_state_dir: &Path,
    activation_vars: &HashMap<String, String>,
    profile_env: &HashMap<String, String>,
    envrc_vars: &HashMap<String, String>,
    hook_path: &Path,
) -> Result<()> {
    let flox_activations_bin = (*FLOX_ACTIVATIONS_BIN).clone();
    let end_json_path = start_state_dir.join(ENV_DIFF_END_JSON);

    // Minimal bash script: source the hook and capture final env.
    let bash_script = format!(
        r#"set +euo pipefail
source "{hook}" 1>&2
set -euo pipefail
"{activations}" dump-env -o "{end_json}""#,
        hook = hook_path.display(),
        activations = flox_activations_bin.display(),
        end_json = end_json_path.display(),
    );

    let mut command = Command::new("bash");
    command.args(["-c", &bash_script]);

    // Apply all pre-computed env vars so the hook runs in the correct environment
    command.envs(activation_vars);
    command.envs(profile_env);
    command.envs(envrc_vars);

    debug!("spawning minimal bash for hook-on-activate");
    let status = {
        let _span = info_span!("run_hook_on_activate").entered();
        command.spawn()?.wait()?
    };

    if !status.success() {
        bail!("Running hook.on-activate failed");
    }

    if !end_json_path.exists() {
        bail!(indoc! {"
            The hook.on-activate script did not complete normally.

            Review your script for the use of:
            - 'exit' commands, which should be replaced with 'return'
            - 'exec' commands, which should be run in a subshell: '(exec command)'"});
    }

    Ok(())
}

/// Write an environment variable HashMap to a JSON file with 0600 permissions.
/// Uses restrictive permissions since env snapshots may contain secrets.
fn write_env_json(path: &Path, env: &HashMap<String, String>) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    let mut writer = std::io::BufWriter::new(file);
    serde_json::to_writer(&mut writer, env)?;
    writer.write_all(b"\n")?;
    Ok(())
}

/// Start services with a new process-compose instance.
///
/// The CLI has already decided that a new process-compose is needed.
/// This function starts process-compose and then starts the specified services.
pub fn start_services_with_new_process_compose(
    activation_state_dir: &Path,
    process_compose_bin: &Path,
    socket_path: &Path,
    services: &[String],
) -> Result<(), anyhow::Error> {
    let activations_json_path = state_json_path(activation_state_dir);
    let (activations_opt, lock) = read_activations_json(&activations_json_path)?;
    let activations = activations_opt.expect("state.json should exist");
    let executive_pid = activations.executive_pid();
    // Don't hold a lock because the executive will need it when starting `process-compose`
    drop(lock);

    debug!("starting new process-compose for services");
    signal_new_process_compose(process_compose_bin, socket_path, executive_pid)?;
    start_services_via_socket(process_compose_bin, socket_path, services)?;

    Ok(())
}

/// Start a new process-compose instance by signaling the executive.
fn signal_new_process_compose(
    process_compose_bin: &Path,
    socket_path: &Path,
    executive_pid: i32,
) -> Result<(), anyhow::Error> {
    // Stop first, if running, to ensure that we wait on the socket from the new instance.
    if socket_path.exists() {
        debug!("shutting down old process-compose");
        if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
            error!(%err, "failed to stop process-compose");
        }
    }

    debug!(
        executive_pid,
        "sending SIGUSR1 to executive to start new process-compose",
    );
    kill(Pid::from_raw(executive_pid), Signal::SIGUSR1)?;

    let activation_timeout = std::env::var("_FLOX_SERVICES_ACTIVATE_TIMEOUT")
        .ok()
        .and_then(|t| t.parse().ok())
        .map(Duration::from_secs_f64)
        .unwrap_or(Duration::from_secs(2));
    let socket_ready = wait_for_socket_ready(process_compose_bin, socket_path, activation_timeout)?;
    if !socket_ready {
        // TODO: We used to print the services log (if it exists) here to
        // help users debug the failure but we no longer have the path
        // available now that it's started by the executive.
        bail!("Failed to start services: process-compose socket not ready");
    }

    Ok(())
}

fn spawn_executive(
    attach: &AttachCtx,
    project: &AttachProjectCtx,
    activation_state_dir: &Path,
    start_state_dir: &Path,
) -> Result<Pid, anyhow::Error> {
    let parent_pid = getpid();

    let executive_ctx = ExecutiveCtx {
        attach_ctx: attach.clone(),
        project_ctx: project.clone(),
        activation_state_dir: activation_state_dir.to_path_buf(),
        parent_pid: parent_pid.as_raw(),
    };

    let temp_file = tempfile::NamedTempFile::with_prefix_in("executive_ctx_", start_state_dir)?;
    serde_json::to_writer(&temp_file, &executive_ctx)?;
    let executive_ctx_path = temp_file.path().to_path_buf();
    temp_file.keep()?;

    // Spawn executive
    let mut executive = Command::new((*FLOX_ACTIVATIONS_BIN).clone());
    executive.args([
        "executive",
        // This is ony provided for the purpose of humans identifying the
        // process from args.
        "--dot-flox-path",
        &project.dot_flox_path.to_string_lossy(),
        "--executive-ctx",
        &executive_ctx_path.to_string_lossy(),
    ]);
    executive
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    debug!(
        "Spawning executive process to start activation: {:?}",
        executive
    );
    let child = executive.spawn()?;
    Ok(Pid::from_raw(child.id() as i32))
}

/// Wait for the executive to signal that it has started by sending SIGUSR1.
/// If the executive dies, then we error.
/// Signals should have been registered for SIGCHLD and SIGUSR1
fn wait_for_executive(child_pid: Pid, mut signals: Signals) -> Result<(), anyhow::Error> {
    debug!(
        "Awaiting SIGUSR1 from child process with PID: {}",
        child_pid
    );

    // I think the executive will always either successfully send SIGUSR1,
    // or it will exit sending SIGCHLD
    // If I'm wrong, this will loop forever
    loop {
        let pending = signals.wait();
        // We want to handle SIGUSR1 rather than SIGCHLD if both
        // are received
        // I'm not 100% confident SIGCHLD couldn't be delivered prior to
        // SIGUSR1 or SIGUSR2,
        // but I haven't seen that since switching to signals.wait() instead
        // of signals.forever()
        // If that does happen, the user would see
        // "Error: Activation process {} terminated unexpectedly"
        // which isn't a huge problem
        let signals = pending.collect::<Vec<_>>();
        // Proceed after receiving SIGUSR1
        if signals.contains(&SIGUSR1) {
            debug!(
                "Received SIGUSR1 (executive started successfully) from child process {}",
                child_pid
            );
            return Ok(());
        } else if signals.contains(&SIGCHLD) {
            // SIGCHLD can come from any child process, not just ours.
            // Use waitpid with WNOHANG to check if OUR child has exited.
            match waitpid(child_pid, Some(WaitPidFlag::WNOHANG)) {
                Ok(WaitStatus::StillAlive) => {
                    // Our child is still alive, SIGCHLD was from a different process
                    debug!(
                        "Received SIGCHLD but child {} is still alive, continuing to wait",
                        child_pid
                    );
                    continue;
                },
                Ok(status) => {
                    // Our child has exited
                    return Err(anyhow!(
                        // TODO: we should print the path to the log file
                        "Executive {} terminated unexpectedly with status: {:?}",
                        child_pid,
                        status
                    ));
                },
                Err(nix::errno::Errno::ECHILD) => {
                    // Child already reaped, this shouldn't happen but handle gracefully
                    return Err(anyhow!(
                        "Executive {} terminated unexpectedly (already reaped)",
                        child_pid
                    ));
                },
                Err(e) => {
                    // Unexpected error from waitpid
                    return Err(anyhow!(
                        "Failed to check status of executive process {}: {}",
                        child_pid,
                        e
                    ));
                },
            }
        } else {
            unreachable!("Received unexpected signal or empty iterator over signals");
        }
    }
}
