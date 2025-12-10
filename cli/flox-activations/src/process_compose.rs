use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};
use std::{env, result, thread};

use anyhow::{Context, Error, bail};
use flox_core::activate::context::ActivateCtx;
use flox_core::activations::{
    EnvForProcessCompose,
    activations_json_path,
    read_activations_json,
    write_activations_json,
};
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::debug;

use crate::activate_script_builder::apply_activation_env;
use crate::cli::activate::VarsFromEnvironment;
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;

/// Path to the process-compose binary
/// TODO: we don't want the dependency here
static PROCESS_COMPOSE_BIN: LazyLock<String> = LazyLock::new(|| {
    std::env::var("PROCESS_COMPOSE_BIN").unwrap_or(env!("PROCESS_COMPOSE_BIN").to_string())
});
const BASH_BIN: &str = env!("X_BASH_BIN");

/// Set env_for_process_compose to Starting in activations.json
fn set_env_for_process_compose_starting(runtime_dir: &str, flox_env: &str) -> Result<(), Error> {
    let activations_path = activations_json_path(runtime_dir, flox_env);
    let (activations, lock) = read_activations_json(&activations_path)?;
    let Some(activations) = activations else {
        bail!("bad state: shouldn't be starting services when activations.json doesn't exist");
    };
    let mut activations = activations.check_version()?;
    let env_for_process_compose = activations.env_for_process_compose_mut();

    match env_for_process_compose {
        Some(EnvForProcessCompose::Started(_)) => {
            bail!("cannot start services when another process already started them");
        },
        Some(env_for_process_compose @ EnvForProcessCompose::Starting(_)) => {
            // Allow restarting if the previous starting process is no longer running
            if env_for_process_compose.still_starting() {
                bail!("cannot start services when another process is simultaneously starting them");
            }
            debug!(
                "overwriting Starting env_for_process_compose entry since pid {} is no longer running",
                std::process::id()
            );
        },
        None => {},
    }

    *env_for_process_compose = Some(EnvForProcessCompose::Starting(std::process::id() as i32));
    write_activations_json(&activations, &activations_path, lock)?;
    Ok(())
}

/// Set env_for_process_compose to Started(store_path) in activations.json
fn set_env_for_process_compose_started(
    runtime_dir: &str,
    flox_env: &str,
    store_path: &str,
) -> Result<(), Error> {
    let activations_path = activations_json_path(runtime_dir, flox_env);
    let (activations, lock) = read_activations_json(&activations_path)?;
    let Some(activations) = activations else {
        bail!(
            "bad state: shouldn't be marking services started when activations.json doesn't exist"
        );
    };
    let mut activations = activations.check_version()?;
    let env_for_process_compose = activations.env_for_process_compose_mut();

    if !matches!(
        env_for_process_compose,
        Some(EnvForProcessCompose::Starting(_))
    ) {
        bail!("bad state: expected env_for_process_compose to be Starting");
    }

    *env_for_process_compose = Some(EnvForProcessCompose::Started(store_path.to_string()));
    write_activations_json(&activations, &activations_path, lock)?;
    Ok(())
}

/// Set env_for_process_compose to None
fn clear_env_for_process_compose(runtime_dir: &str, flox_env: &str) -> Result<(), Error> {
    let activations_path = activations_json_path(runtime_dir, flox_env);
    let (activations, lock) = read_activations_json(&activations_path)?;
    let Some(activations) = activations else {
        bail!("bad state: shouldn't be starting when activations.json doesn't exist");
    };
    let mut activations = activations.check_version()?;
    let env_for_process_compose = activations.env_for_process_compose_mut();

    *env_for_process_compose = None;
    write_activations_json(&activations, &activations_path, lock)?;
    Ok(())
}

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
    context: &ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach: &StartOrAttachResult,
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
        start_or_attach,
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

    // Set state to starting immediately before spawning process-compose
    set_env_for_process_compose_starting(&context.flox_runtime_dir, &context.env)?;

    let result = try_start_services_blocking(command, socket_file, log_file);

    if let Err(e) = result {
        let result = clear_env_for_process_compose(&context.flox_runtime_dir, &context.env);
        if let Err(cleanup_err) = result {
            eprintln!(
                "Failed to clear env_for_process_compose after failed start: {:?}",
                cleanup_err
            );
        }
        return Err(e);
    }

    // Update activations.json to mark process-compose as Started with the store path
    set_env_for_process_compose_started(
        &context.flox_runtime_dir,
        &context.env,
        &context.flox_activate_store_path,
    )?;

    Ok(())
}

/// Wrapping function for starting process-compose and blocking so we can catch
/// all errors and cleanup activations.json on failure
fn try_start_services_blocking(
    mut command: Command,
    socket_file: &PathBuf,
    log_file: PathBuf,
) -> Result<(), Error> {
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
