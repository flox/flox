use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Error, bail};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx, SandboxMode};
use flox_core::activations::StartIdentifier;
use flox_core::process_compose::PROCESS_NEVER_EXIT_NAME;
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::{debug, info};

use crate::attach_diff::AttachDiff;
use crate::start_diff::StartDiff;
use crate::vars_from_env::VarsFromEnvironment;

const BASH_BIN: &str = env!("X_BASH_BIN");

/// Wait for the process-compose socket to become ready.
///
/// Returns `true` if socket is ready, `false` if timeout, and Error for other failures.
pub fn wait_for_socket_ready(
    process_compose_bin: &Path,
    socket_file: &Path,
    timeout: Duration,
) -> Result<bool, Error> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(20);

    let mut command = Command::new(process_compose_bin);
    command
        .env("NO_COLOR", "1")
        .arg("process")
        .arg("list")
        .arg("-u")
        .arg(socket_file)
        .arg("-o")
        .arg("json");
    let pretty_command = format!("{:?}", command);
    debug!(
        "Beginning polling socket readiness with command: {}",
        pretty_command
    );

    loop {
        let output = command.output().context(format!(
            "failed to poll socket readiness with command: {pretty_command}"
        ))?;

        // Ignore command status and just check if we get valid JSON
        let result: Result<Vec<serde_json::Value>, _> = serde_json::from_slice(&output.stdout);
        if let Ok(parsed) = result
            && !parsed.is_empty()
        {
            let status = parsed[0].get("status");
            if status.is_some() {
                return Ok(true);
            }
        }

        if start.elapsed() >= timeout {
            return Ok(false);
        }

        thread::sleep(poll_interval);
    }
}

/// Return a copy of `ctx` with the sandbox mode cleared to `Off`.
///
/// Services run unsandboxed in the prototype: their process-compose
/// supervisor (and every process it spawns) must not inherit the sandbox
/// preload or policy. Because `double_set_envs` keys the sandbox injection
/// off `sandbox_mode`, forcing it to `Off` here is what strips all sandbox
/// vars from the services `AttachDiff`.
fn unsandboxed_ctx(ctx: &AttachCtx) -> AttachCtx {
    AttachCtx {
        sandbox_mode: SandboxMode::Off,
        ..ctx.clone()
    }
}

/// Start process-compose with only the flox_never_exit service.
/// This allows services to be started later via the socket API.
pub fn start_process_compose_no_services(
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project: &AttachProjectCtx,
    start_id: &StartIdentifier,
    activation_state_dir: &Path,
) -> Result<(), Error> {
    let start_state_dir = start_id.start_state_dir(activation_state_dir)?;
    let config_file = start_id.store_path.join("service-config.yaml");
    let socket_path = project.flox_services_socket.as_path();

    // Generate timestamped log file name
    let format =
        format_description!("[year][month][day][hour][minute][second][subsecond digits:6]");
    let timestamp = OffsetDateTime::now_local()?
        .format(&format)
        .context("failed to format timestamp")?;
    let log_file = project
        .flox_env_log_dir
        .join(format!("services.{}.log", timestamp));

    let mut command = Command::new(&project.process_compose_bin);

    // The executive inherits the pre-activation environment from activate,
    // so these values are the same as what the initial activation captured.
    let vars_from_env = VarsFromEnvironment::get()?;
    // Load the environment diff for the activation that we're attaching to.
    let start_diff = StartDiff::from_files(&start_state_dir)?;
    // Services run unsandboxed in the prototype: clear the sandbox mode on
    // the context used for the process-compose AttachDiff so none of the
    // sandbox preload/policy vars are injected into the services supervisor
    // or the processes it spawns. Sandboxing services is intentionally out of
    // scope here.
    let services_attach_ctx = unsandboxed_ctx(attach_ctx);
    let attach_diff = AttachDiff::new(
        &services_attach_ctx,
        Some(project),
        subsystem_verbosity,
        vars_from_env,
        &start_diff,
        false,
    )?;
    attach_diff.apply_to_command(&mut command);

    command
        .env("NO_COLOR", "1")
        .env("COMPOSE_SHELL", BASH_BIN)
        .arg("up")
        .arg("-f")
        .arg(&config_file)
        .arg("-u")
        .arg(socket_path)
        .arg("-L")
        .arg(&log_file)
        .arg("--disable-dotenv")
        .arg("--tui=false")
        .arg(PROCESS_NEVER_EXIT_NAME); // Only start the never_exit service

    // Redirect stdio to detach from terminal
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    info!(
        ?config_file,
        ?socket_path,
        ?log_file,
        "spawning process-compose without any services: {:?}",
        command
    );
    command.spawn().context("Failed to spawn process-compose")?;

    Ok(())
}

/// Start specific services via the process-compose socket API.
/// This should be called after process-compose is ready.
pub fn start_services_via_socket(
    process_compose_bin: &Path,
    socket_path: &Path,
    services: &[String],
) -> Result<(), Error> {
    for service in services {
        if service == PROCESS_NEVER_EXIT_NAME {
            continue;
        }

        let mut cmd = Command::new(process_compose_bin);
        cmd.env("NO_COLOR", "1")
            .arg("--unix-socket")
            .arg(socket_path)
            .arg("process")
            .arg("start")
            .arg(service);

        debug!(service, ?cmd, "starting service via socket");

        let output = cmd
            .output()
            .context(format!("failed to start service '{}'", service))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Ignore "already running" errors
            if !stderr.contains("is already running") {
                bail!("Failed to start service '{}': {}", service, stderr);
            }
        }
    }

    Ok(())
}

/// Shuts down process-compose by running `process-compose down` via the unix socket.
pub fn process_compose_down(process_compose_bin: &Path, socket_path: &Path) -> Result<(), Error> {
    let mut cmd = Command::new(process_compose_bin);
    cmd.arg("down");
    cmd.arg("--unix-socket");
    cmd.arg(socket_path);
    cmd.env("NO_COLOR", "1");

    debug!(
        command = format!(
            "{} down --unix-socket {}",
            process_compose_bin.display(),
            socket_path.display()
        ),
        "running process-compose down"
    );

    let output = cmd
        .output()
        .context("failed to execute process-compose down")?;

    output.status.success().then_some(()).ok_or_else(|| {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::anyhow!("process-compose down failed: {}", stderr)
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::attach_diff::double_set_envs;
    use crate::sandbox::{
        FLOX_SANDBOX_ALLOW_DIRS_VAR,
        FLOX_SANDBOX_ALLOW_VAR,
        FLOX_SANDBOX_GRANTS_DIR_VAR,
        FLOX_SRC_DIR_VAR,
        FLOX_VIRTUAL_SANDBOX_VAR,
        PRELOAD_VAR,
    };

    fn sandboxed_ctx() -> AttachCtx {
        AttachCtx {
            env: "/flox_env".to_string(),
            env_cache: PathBuf::from("/cache"),
            env_description: "test".to_string(),
            flox_active_environments: "[]".to_string(),
            prompt_color_1: "1".to_string(),
            prompt_color_2: "2".to_string(),
            flox_prompt_environments: "".to_string(),
            set_prompt: false,
            flox_env_cuda_detection: "0".to_string(),
            interpreter_path: PathBuf::from("/interpreter"),
            sandbox_mode: SandboxMode::Enforce,
        }
    }

    #[test]
    fn unsandboxed_ctx_clears_mode() {
        let ctx = sandboxed_ctx();
        assert_eq!(ctx.sandbox_mode, SandboxMode::Enforce);
        let cleared = unsandboxed_ctx(&ctx);
        assert_eq!(cleared.sandbox_mode, SandboxMode::Off);
        // Everything else is preserved.
        assert_eq!(cleared.env, ctx.env);
        assert_eq!(cleared.interpreter_path, ctx.interpreter_path);
    }

    /// The services AttachDiff is built from `unsandboxed_ctx`, so it must
    /// carry none of the sandbox vars even when the original activation was
    /// sandboxed. No project context is needed: an Off-mode context never
    /// reaches the library-resolution path, so the diff is sandbox-free
    /// regardless.
    #[test]
    fn services_diff_carries_no_sandbox_vars() {
        let ctx = sandboxed_ctx();
        let services_ctx = unsandboxed_ctx(&ctx);
        let diff = double_set_envs(&services_ctx, None);
        for name in [
            FLOX_VIRTUAL_SANDBOX_VAR,
            FLOX_SANDBOX_ALLOW_VAR,
            FLOX_SANDBOX_ALLOW_DIRS_VAR,
            FLOX_SRC_DIR_VAR,
            FLOX_SANDBOX_GRANTS_DIR_VAR,
            PRELOAD_VAR,
        ] {
            assert!(
                !diff.additions.contains_key(name),
                "services diff must not inject {name}"
            );
        }
    }
}
