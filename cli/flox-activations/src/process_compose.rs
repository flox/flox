use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Error, bail};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
use flox_core::activations::StartIdentifier;
use flox_core::process_compose::PROCESS_NEVER_EXIT_NAME;
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::{debug, info};

use crate::activate_script_builder::apply_activation_env;
use crate::env_diff::EnvDiff;
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
    let env_diff = EnvDiff::from_files(&start_state_dir)?;
    apply_activation_env(
        &mut command,
        attach_ctx,
        Some(project),
        subsystem_verbosity,
        vars_from_env,
        &env_diff,
        &start_state_dir,
    );

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
        "spawning process-compose without any services",
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
