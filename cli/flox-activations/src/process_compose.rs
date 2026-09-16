use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Error, bail};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
use flox_core::activations::StartIdentifier;
use flox_core::process_compose::{PROCESS_NEVER_EXIT_NAME, manager_responds};
use time::OffsetDateTime;
use time::macros::format_description;
use tracing::{debug, info};

use crate::attach_diff::AttachDiff;
use crate::env_trace::EnvTrace;
use crate::vars_from_env::VarsFromEnvironment;

const BASH_BIN: &str = env!("X_BASH_BIN");

/// Wait for a `process-compose` manager to start answering on `socket_file`.
///
/// Returns `true` once one answers, `false` on timeout.
///
/// `process-compose` does not bind its API socket until the project is loaded,
/// so an answer here is also proof that services can be started through it.
pub fn wait_for_socket_ready(socket_file: &Path, timeout: Duration) -> bool {
    let start = Instant::now();
    let poll_interval = Duration::from_millis(20);

    debug!(?socket_file, "polling for a process-compose manager");

    loop {
        if manager_responds(socket_file) {
            return true;
        }

        if start.elapsed() >= timeout {
            return false;
        }

        thread::sleep(poll_interval);
    }
}

/// A stamp of the current time comparable with the ones in log names.
///
/// One definition for both sides: the executive names the log it writes with
/// this, and a caller waiting on that manager bounds its search with it.
pub fn log_timestamp_now() -> Result<String, Error> {
    let format =
        format_description!("[year][month][day][hour][minute][second][subsecond digits:6]");
    OffsetDateTime::now_local()?
        .format(&format)
        .context("failed to format timestamp")
}

/// The most recent `services.*.log` in `log_dir` stamped after `not_before`.
///
/// Compares timestamped names rather than mtimes, which keeps this independent
/// of filesystems with coarse or skewed timestamps. The names are fixed width
/// and zero padded, so comparing them as text orders them by time.
///
/// `not_before` is what makes the newest log the right one rather than merely
/// the latest: a manager that died before writing one would otherwise leave
/// the previous start's log to be quoted as the account of this failure.
pub fn latest_services_log(log_dir: &Path, not_before: Option<&str>) -> Option<PathBuf> {
    std::fs::read_dir(log_dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let Some(stamp) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_prefix("services."))
                .and_then(|name| name.strip_suffix(".log"))
            else {
                return false;
            };
            not_before.is_none_or(|floor| stamp > floor)
        })
        .max()
}

/// The last `lines` lines of `path`, or `None` if it is missing or empty.
pub fn log_tail(path: &Path, lines: usize) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let tail = contents
        .lines()
        .rev()
        .take(lines)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    (!tail.trim().is_empty()).then_some(tail)
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

    let log_file = project
        .flox_env_log_dir
        .join(format!("services.{}.log", log_timestamp_now()?));

    let mut command = Command::new(&project.process_compose_bin);

    // The executive inherits the pre-activation environment from activate,
    // so these values are the same as what the initial activation captured.
    let vars_from_env = VarsFromEnvironment::get()?;
    // Load the environment trace for the activation that we're attaching to.
    let env_trace = EnvTrace::from_state_dir(&start_state_dir)?;
    let attach_diff = AttachDiff::new(
        attach_ctx,
        Some(project),
        subsystem_verbosity,
        vars_from_env,
        &env_trace,
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
    use super::*;

    /// Stand in for a manager: answer `GET /live` once, then stop.
    ///
    /// Reads the request before replying, as any real server does. Closing a
    /// socket with unread data in its receive buffer sends an RST on Linux,
    /// which discards the reply that was just written.
    fn serve_live(listener: std::os::unix::net::UnixListener) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        })
    }

    #[test]
    fn readiness_waits_for_a_manager_to_answer() {
        let dir = tempfile::Builder::new()
            .prefix("pcsock")
            .tempdir_in("/tmp")
            .unwrap();
        let socket = dir.path().join("s.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = serve_live(listener);

        assert!(wait_for_socket_ready(&socket, Duration::from_secs(5)));
        server.join().unwrap();
    }

    /// A socket file that no manager is behind must not read as ready; that
    /// conflation is the bug this guards.
    #[test]
    fn readiness_times_out_on_a_stale_socket() {
        let dir = tempfile::Builder::new()
            .prefix("pcsock")
            .tempdir_in("/tmp")
            .unwrap();
        let stale = dir.path().join("s.sock");

        // Binding creates the file; dropping the listener leaves it behind.
        drop(std::os::unix::net::UnixListener::bind(&stale).unwrap());
        assert!(stale.exists(), "the socket file must outlive its listener");

        assert!(!wait_for_socket_ready(&stale, Duration::from_millis(100)));
    }

    #[test]
    fn latest_services_log_picks_the_newest_timestamped_name() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "services.20260825120000000000.log",
            "services.20260825120001000000.log",
            "services.20260824235959000000.log",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }

        assert_eq!(
            latest_services_log(dir.path(), None),
            Some(dir.path().join("services.20260825120001000000.log"))
        );
    }

    #[test]
    fn latest_services_log_ignores_other_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("services.log.old"), "").unwrap();
        std::fs::write(dir.path().join("activation.log"), "").unwrap();

        assert_eq!(latest_services_log(dir.path(), None), None);
    }

    /// The point of the bound: a manager that died before writing a log must
    /// not have the previous start's log quoted as the account of its failure.
    #[test]
    fn latest_services_log_ignores_logs_older_than_the_floor() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("services.20260825120000000000.log"),
            "from an earlier start",
        )
        .unwrap();

        assert_eq!(
            latest_services_log(dir.path(), Some("20260825130000000000")),
            None
        );
    }

    #[test]
    fn latest_services_log_takes_a_log_written_after_the_floor() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "services.20260825120000000000.log",
            "services.20260825140000000000.log",
        ] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }

        assert_eq!(
            latest_services_log(dir.path(), Some("20260825130000000000")),
            Some(dir.path().join("services.20260825140000000000.log"))
        );
    }

    /// Both sides derive the name from one definition, so a stamp taken here
    /// is comparable with the ones the executive writes.
    #[test]
    fn log_timestamp_is_comparable_with_a_log_name() {
        let dir = tempfile::tempdir().unwrap();
        let floor = log_timestamp_now().unwrap();
        // Stamps are microsecond resolution, so two calls can tie; let the
        // clock advance rather than race it.
        thread::sleep(Duration::from_millis(2));
        let later = log_timestamp_now().unwrap();
        std::fs::write(dir.path().join(format!("services.{later}.log")), "").unwrap();

        assert_eq!(
            latest_services_log(dir.path(), Some(&floor)),
            Some(dir.path().join(format!("services.{later}.log")))
        );
    }

    #[test]
    fn latest_services_log_without_a_log_dir() {
        assert_eq!(
            latest_services_log(Path::new("/does_not_exist"), None),
            None
        );
    }

    #[test]
    fn log_tail_returns_the_last_lines() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("services.log");
        std::fs::write(&log, "one\ntwo\nthree\nfour\n").unwrap();

        assert_eq!(log_tail(&log, 2), Some("three\nfour".to_string()));
    }

    #[test]
    fn log_tail_of_an_empty_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("services.log");
        std::fs::write(&log, "\n \n").unwrap();

        assert_eq!(log_tail(&log, 10), None);
    }

    #[test]
    fn log_tail_of_a_missing_log() {
        assert_eq!(log_tail(Path::new("/does_not_exist.log"), 10), None);
    }
}
