use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use std::{env, thread};

use anyhow::{Context, Error, bail};
use flox_core::activate::context::AttachCtx;
use flox_core::activations::StartIdentifier;
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::debug;

use crate::activate_script_builder::apply_activation_env;
use crate::cli::activate::VarsFromEnvironment;
use crate::env_diff::EnvDiff;

/// Path to the process-compose binary
/// TODO: we don't want the dependency here
static PROCESS_COMPOSE_BIN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});
const BASH_BIN: &str = env!("X_BASH_BIN");

/// Wait for the process-compose socket to become ready.
///
/// Returns true if services started, false if they didn't start within timeout,
/// and Error for other failures.
fn wait_for_services_socket(socket_file: &Path, timeout: Duration) -> Result<bool, Error> {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(20);

    let mut command = Command::new(&*PROCESS_COMPOSE_BIN);
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
        "Beginning polling services status with command: {}",
        pretty_command
    );

    loop {
        let output = command.output().context(format!(
            "failed to poll services status with command: {pretty_command}"
        ))?;

        // Ignore command status and just check if we get the JSON we're looking for
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

/// Start services using process-compose, blocking until the socket is ready.
pub fn start_services_blocking(
    context: &AttachCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_id: &StartIdentifier,
    env_diff: EnvDiff,
) -> Result<(), anyhow::Error> {
    let config_file = format!("{}/service-config.yaml", context.env);
    let Some(socket_file) = &context.flox_services_socket else {
        unreachable!("flox_services_socket must be set to start services");
    };
    // Generate timestamped log file name
    let format =
        format_description!("[year][month][day][hour][minute][second][subsecond digits:6]");
    let timestamp = OffsetDateTime::now_local()?
        .format(&format)
        .context("failed to format timestamp")?;
    let Some(flox_env_log_dir) = &context.flox_env_log_dir else {
        unreachable!("flox_env_log_dir must be set to start services");
    };
    let log_file = flox_env_log_dir.join(format!("services.{}.log", timestamp));

    debug!(
        "Starting process-compose with config: {:?}, socket: {:?}, log: {:?}",
        config_file, socket_file, log_file
    );

    // Build the command
    let mut command = Command::new(&*PROCESS_COMPOSE_BIN);
    apply_activation_env(
        &mut command,
        context.clone(),
        subsystem_verbosity,
        vars_from_env,
        &env_diff,
        start_id,
    );
    command
        .env("NO_COLOR", "1")
        .env("COMPOSE_SHELL", BASH_BIN)
        .arg("up")
        .arg("-f")
        .arg(config_file)
        .arg("-u")
        .arg(socket_file)
        .arg("-L")
        .arg(&log_file)
        .arg("--disable-dotenv")
        .arg("--tui=false");

    // Add specific services if provided
    if let Some(services) = &context.flox_services_to_start {
        command.args(services);
    }

    // Redirect stdio to detach from terminal
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    debug!("Spawning process-compose: {:?}", command);
    command.spawn().context("Failed to spawn process-compose")?;

    let activation_timeout = if let Ok(timeout) = env::var("_FLOX_SERVICES_ACTIVATE_TIMEOUT") {
        Duration::from_secs_f64(timeout.parse()?)
    } else {
        Duration::from_secs(2)
    };

    // Wait for the socket to become ready
    let started = wait_for_services_socket(socket_file, activation_timeout)?;
    if !started {
        // Startup failed, return error with log file contents if available
        if !log_file.exists() {
            bail!("Failed to start services");
        } else {
            let log_contents = std::fs::read_to_string(&log_file)
                .unwrap_or_else(|_| format!("unable to read logs in '{}'", log_file.display()));
            bail!("Failed to start services:\n{}", log_contents);
        }
    }

    debug!("Process-compose socket ready at: {:?}", socket_file);
    Ok(())
}
