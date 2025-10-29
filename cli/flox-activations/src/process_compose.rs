/// Process-compose integration for managing services in the executive.
///
/// This module handles starting and stopping the process-compose daemon,
/// as well as communicating with it via the Unix socket to start services.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use log::debug;

/// Path to the process-compose binary
static PROCESS_COMPOSE_BIN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});

/// Path to the sleep binary for flox_never_exit service
static SLEEP_BIN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("SLEEP_BIN").unwrap_or(env!("SLEEP_BIN").to_string())
});

/// Generate a minimal process-compose config with only flox_never_exit.
///
/// This is used when the service-config.yaml doesn't exist (no services defined).
/// Returns the path to the generated config file in a temp location.
fn generate_minimal_config() -> Result<PathBuf> {
    // Create a minimal YAML config with just flox_never_exit
    let config_content = format!(
        r#"version: "0.5"
log_level: error
log_configuration:
  no_metadata: true
disable_env_expansion: true
processes:
  flox_never_exit:
    command: "{sleep_bin} infinity"
    is_daemon: false
"#,
        sleep_bin = *SLEEP_BIN
    );

    // Write to a temp file
    let temp_dir = std::env::temp_dir();
    let config_path = temp_dir.join(format!("flox-minimal-service-config-{}.yaml", std::process::id()));

    fs::write(&config_path, config_content)
        .context("Failed to write minimal service config")?;

    debug!("Generated minimal service config at: {:?}", config_path);

    Ok(config_path)
}

/// Start the process-compose daemon.
///
/// This spawns process-compose in the background with the given config and socket.
/// It waits for the socket file to appear before returning.
///
/// If the config file doesn't exist, a minimal config with just flox_never_exit is generated.
///
/// # Arguments
///
/// * `config_path` - Path to the service-config.yaml file
/// * `socket_path` - Path to the Unix socket for process-compose
/// * `services_to_start` - Optional list of specific services to start
///
/// # Returns
///
/// Returns Ok(()) if process-compose started successfully and the socket appeared.
pub fn start_process_compose(
    config_path: impl AsRef<Path>,
    socket_path: impl AsRef<Path>,
    services_to_start: Option<&[String]>,
) -> Result<()> {
    let config_path = config_path.as_ref();
    let socket_path = socket_path.as_ref();

    // Check if config file exists, if not generate a minimal one
    let actual_config_path: PathBuf = if config_path.exists() {
        debug!("Using existing service config: {:?}", config_path);
        config_path.to_path_buf()
    } else {
        debug!("Service config not found at {:?}, generating minimal config", config_path);
        generate_minimal_config()?
    };

    debug!(
        "Starting process-compose with config: {:?}, socket: {:?}",
        actual_config_path, socket_path
    );

    // Build the command
    let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);
    cmd.env("NO_COLOR", "1")
        .arg("--unix-socket")
        .arg(socket_path)
        .arg("--config")
        .arg(&actual_config_path)
        .arg("--tui=false")
        .arg("up");

    // Add specific services if provided
    if let Some(services) = services_to_start {
        cmd.args(services);
    }

    // Redirect stdio to null to daemonize
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // Spawn the process
    let child = cmd
        .spawn()
        .context("Failed to spawn process-compose")?;

    debug!("Process-compose spawned with PID: {}", child.id());

    // Wait for the socket to appear
    wait_for_socket(socket_path)?;

    debug!("Process-compose socket appeared at: {:?}", socket_path);

    Ok(())
}

/// Wait for the process-compose socket to appear.
///
/// Polls for the socket file to exist, with exponential backoff.
fn wait_for_socket(socket_path: &Path) -> Result<()> {
    const MAX_TRIES: u64 = 10;

    for attempt in 1..=MAX_TRIES {
        if socket_path.exists() {
            return Ok(());
        }

        // Exponential backoff: 100ms, 200ms, 400ms, etc.
        let delay = Duration::from_millis(100 * attempt);
        debug!(
            "Socket not ready, waiting {}ms (attempt {}/{})",
            delay.as_millis(),
            attempt,
            MAX_TRIES
        );
        thread::sleep(delay);
    }

    Err(anyhow!(
        "Socket did not appear after {} attempts: {:?}",
        MAX_TRIES,
        socket_path
    ))
}

/// Stop the process-compose daemon.
///
/// Sends a "down" command to the process-compose socket to gracefully shut it down.
///
/// # Arguments
///
/// * `socket_path` - Path to the Unix socket for process-compose
pub fn stop_process_compose(socket_path: impl AsRef<Path>) -> Result<()> {
    let socket_path = socket_path.as_ref();

    debug!("Stopping process-compose at socket: {:?}", socket_path);

    // Check if socket exists before trying to stop
    if !socket_path.exists() {
        debug!("Socket doesn't exist, process-compose may have already stopped");
        return Ok(());
    }

    let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);
    cmd.env("NO_COLOR", "1")
        .arg("down")
        .arg("--unix-socket")
        .arg(socket_path);

    let output = cmd.output().context("Failed to execute process-compose down")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!("process-compose down failed: {}", stderr);
        // Don't fail hard here - the daemon might already be down
    } else {
        debug!("Process-compose stopped successfully");
    }

    Ok(())
}

/// Start specific services via the process-compose socket.
///
/// # Arguments
///
/// * `socket_path` - Path to the Unix socket for process-compose
/// * `service_names` - Names of services to start
pub fn start_services(
    socket_path: impl AsRef<Path>,
    service_names: &[String],
) -> Result<()> {
    let socket_path = socket_path.as_ref();

    if service_names.is_empty() {
        return Ok(());
    }

    debug!(
        "Starting services via socket {:?}: {:?}",
        socket_path, service_names
    );

    for service_name in service_names {
        let mut cmd = Command::new(&*PROCESS_COMPOSE_BIN);
        cmd.env("NO_COLOR", "1")
            .arg("--unix-socket")
            .arg(socket_path)
            .arg("process")
            .arg("start")
            .arg(service_name);

        let output = cmd
            .output()
            .context(format!("Failed to start service: {}", service_name))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow!(
                "Failed to start service '{}': {}",
                service_name,
                stderr
            ));
        }

        debug!("Started service: {}", service_name);
    }

    Ok(())
}
