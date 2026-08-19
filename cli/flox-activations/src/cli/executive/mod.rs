use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use clap::Args;
use event_coordinator::{EventCoordinator, ExecutiveEvent};
use flox_core::activate::context::{AttachCtx, AttachProjectCtx};
use flox_core::activate::vars::FLOX_EXECUTIVE_VERBOSITY_VAR;
use flox_core::activations::{
    acquire_activations_json_lock,
    read_activations_json,
    state_json_path,
    write_activations_json,
};
use flox_core::proc_status::read_pid_status;
use flox_core::sentry::init_sentry;
use flox_core::traceable_path;
use fslock::LockFile;
use log_gc::{spawn_heartbeat_log, spawn_logs_gc_threads};
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, getpgid, getpid, setsid};
use reaper::reap_orphaned_children;
use serde::{Deserialize, Serialize};
use tracing::{debug, debug_span, error, info, instrument, warn};
use uuid::Uuid;
use watcher::LockedActivationState;

use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::logger::init_executive_logger;
use crate::on_deactivate::sweep_orphaned_starts;
use crate::process_compose::{process_compose_down, start_process_compose_no_services};

mod event_coordinator;
mod log_gc;
mod reaper;
mod watcher;

#[cfg(target_os = "linux")]
use reaper::linux::SubreaperGuard;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub attach_ctx: AttachCtx,
    pub project_ctx: AttachProjectCtx,
    pub activation_state_dir: std::path::PathBuf,
    pub parent_pid: i32,
    /// The metrics UUID for Sentry user identification.
    /// When Some, Sentry is initialized with this user ID.
    /// When None, metrics are disabled and Sentry is not initialized.
    #[serde(default)]
    pub metrics_uuid: Option<Uuid>,
}

#[derive(Debug, Args)]
pub struct ExecutiveArgs {
    /// .flox directory path
    // This isn't consumed and serves only to identify in process listings which
    // environment the executive is responsible for.
    #[arg(long)]
    pub dot_flox_path: PathBuf,

    /// Path to JSON file containing executive context
    #[arg(long)]
    pub executive_ctx: PathBuf,
}

impl ExecutiveArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        // Step 1: Extract context which we need to do anything.
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            attach_ctx,
            project_ctx,
            activation_state_dir,
            parent_pid,
            metrics_uuid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        // Step 2: Setup logger, so that we can record errors.
        let log_dir = project_ctx.flox_env_log_dir.clone();
        let log_file = format!("executive.{}.log", std::process::id());
        // Read verbosity from dedicated executive variable, not `activate -v`
        // Only takes numeric values like its `FLOX_ACTIVATIONS_VERBOSITY` counterpart.
        let subsystem_verbosity = std::env::var(FLOX_EXECUTIVE_VERBOSITY_VAR)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        init_executive_logger(subsystem_verbosity, log_file, &log_dir)
            .context("failed to initialize logger")?;

        // Step 3: Setup root span with PID, so that logs contain PID.
        // We can set this eagerly because the PID doesn't change after this entry
        // point. Re-execs of activate->executive will cross this entry point again.
        let pid = std::process::id();
        let _root_span = debug_span!("flox_activations::executive", pid = pid).entered();
        info!("{self:?}");

        // Step 4: Setup Sentry, so that we get exception reports.
        // Skip if metrics_uuid not present (metrics disabled)
        let _sentry_guard =
            metrics_uuid.and_then(|uuid| init_sentry("flox-activations::executive", uuid));

        // Step 5: Catch errors from sub-reaper and setsid.

        // Set as subreaper. The guard ensures cleanup on all exit paths.
        #[cfg(target_os = "linux")]
        let _subreaper_guard = SubreaperGuard::new()?;

        // Ensure the executive is detached from the terminal
        ensure_process_group_leader()
            .context("failed to ensure executive is detached from terminal")?;

        // Step 6: Set up signal handlers (among other watchers).
        // All signals registered together.
        let state_json_path = state_json_path(&activation_state_dir);
        let mut coordinator =
            EventCoordinator::new().context("failed to create event coordinator")?;
        coordinator.spawn_all_watchers(state_json_path)?;

        // Step 7: Signal SIGUSR1 when all setup and possible errors have passed.
        info!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        // Step 8: Spawn non-essential GC threads
        spawn_heartbeat_log();
        spawn_logs_gc_threads(&log_dir);

        // Step 9: Enter the monitoring loop
        info!("starting monitoring loop");
        let result = run_event_loop(
            attach_ctx,
            project_ctx,
            activation_state_dir,
            coordinator,
            subsystem_verbosity,
        );
        info!("executive exiting: {:?}", &result);
        result
    }
}

/// Ensures the executive is detached from the terminal by becoming a process group leader.
///
/// We want to make sure that the executive is detached from the terminal in case it sends
/// any signals to the activation. A terminal sends signals to all processes in a process group,
/// and we want to make sure that the executive is in its own process group to avoid receiving any
/// signals intended for the shell.
///
/// From local testing I haven't been able to deliver signals to the executive by sending signals to
/// the activation, so this is more of a "just in case" measure.
fn ensure_process_group_leader() -> Result<(), anyhow::Error> {
    let pid = getpid();
    // Trivia:
    // You can't create a new session if you're already a session leader, the reason being that
    // the other processes in the group aren't automatically moved to the new session. You're supposed
    // to have this invariant: all processes in a process group share the same controlling terminal.
    // If you were able to create a new session as session leader and leave behind the other processes
    // in the group in the old session, it would be possible for processes in this group to be in two
    // different sessions and therefore have two different controlling terminals.
    if pid != getpgid(None).context("failed to get process group leader")? {
        setsid().context("failed to create new session")?;
    }
    Ok(())
}

/// Monitoring loop that responds to ExecutiveEvents and performs cleanup.
///
/// It assumes watchers have already been started for the coordinator.
#[instrument("monitoring", err(Debug), skip_all)]
fn run_event_loop(
    // AttachCtx from when the Executive was started.
    // Does NOT represent the most recent attach.
    initial_attach_ctx: AttachCtx,
    project_ctx: AttachProjectCtx,
    activation_state_dir: PathBuf,
    coordinator: EventCoordinator,
    subsystem_verbosity: u32,
) -> Result<()> {
    let state_json_path = state_json_path(&activation_state_dir);

    let mut loop_guard = LoopGuard::new(5);

    let process_compose_bin = project_ctx.process_compose_bin.to_path_buf();
    let socket_path = project_ctx.flox_services_socket.to_path_buf();
    debug!(
        socket = traceable_path(&socket_path),
        exists = &socket_path.exists(),
        "checked socket"
    );

    // Main event loop - blocks on channel recv.
    //
    // Design note: Only TerminationSignal and ProcessExited can exit the loop,
    // so strictly speaking everything else (SigChld, StartServices, StateFileChanged)
    // could run on its own thread without the coordinator. However, routing all
    // events through the main thread makes it easier to reason about and minimizes
    // races (e.g., what happens if ProcessExited and StateFileChanged happen at
    // the same time).
    loop {
        let event = coordinator.receiver.recv();
        debug!("received from event receiver: {:?}", &event);
        match event {
            Ok(ExecutiveEvent::ProcessExited { pid }) => {
                let should_exit = handle_process_exited(
                    pid,
                    &coordinator,
                    &mut loop_guard,
                    &state_json_path,
                    &process_compose_bin,
                    &socket_path,
                    &activation_state_dir,
                    subsystem_verbosity,
                    &initial_attach_ctx,
                    &project_ctx,
                )?;
                if should_exit {
                    return Ok(());
                }
            },
            Ok(ExecutiveEvent::StartServices) => {
                debug!("Received SIGUSR1, starting process-compose");
                let (activations_json, lock) = read_activations_json(&state_json_path)?;
                let Some(activations) = activations_json else {
                    // TODO: we should probably call cleanup_on_no_state here, but it's
                    // a more complicated situation than other state.json missing cases
                    // because the executive hasn't started services yet.
                    // At the same time it's less likely to be reachable since we should
                    // be here soon after running a CLI command.
                    return Err(anyhow!(
                        "executive shouldn't be running when state.json doesn't exist"
                    )
                    .context("when handling StartServices"))?;
                };

                match handle_start_services_signal(
                    (activations, lock),
                    subsystem_verbosity,
                    &initial_attach_ctx,
                    &project_ctx,
                    &activation_state_dir,
                ) {
                    Ok(Some((activations, lock))) => {
                        write_activations_json(&activations, &state_json_path, lock)?;
                    },
                    Ok(None) => {},
                    Err(err) => {
                        error!(%err, "failed to handle start services signal");
                    },
                }
            },
            Ok(ExecutiveEvent::StateFileChanged) => {
                debug!("state.json changed, checking for new PIDs to monitor");
                let (state, lock) = read_activations_json(&state_json_path)?;
                let Some(mut activations) = state else {
                    // state.json went away between the watcher observing it and
                    // this read. There is nothing left to monitor, so take the
                    // same exit as StateFileRemoved rather than erroring.
                    return cleanup_on_no_state(
                        lock,
                        "handling a state.json change",
                        &state_json_path,
                        &process_compose_bin,
                        &socket_path,
                        &activation_state_dir,
                    );
                };
                coordinator
                    .ensure_monitoring_pids(activations.all_attached_pids_and_expiration())
                    .context("failed to ensure monitoring PIDs")?;
                // When `detach` removes the last PID (e.g., in-place deactivation
                // where the shell process is still alive), trigger cleanup now
                // rather than waiting for the process to exit — which could be a
                // very long time.  This mirrors ProcessExited → watcher::cleanup_pid
                // → attached_pids_is_empty() → cleanup_all, but fires proactively
                // on the state-file change instead.
                if activations.attached_pids_is_empty() {
                    info!("all PIDs gone after state.json change, running cleanup");
                    if cleanup_all(
                        (activations, lock),
                        &process_compose_bin,
                        &socket_path,
                        &activation_state_dir,
                        subsystem_verbosity,
                        &initial_attach_ctx,
                        &project_ctx,
                    )
                    .context("cleanup failed after StateFileChanged with empty PIDs")?
                    {
                        return Ok(());
                    }
                    // A PID raced in between the empty check and cleanup_all;
                    // continue the event loop so the new PID stays monitored.
                } else {
                    // A detach may have emptied a start while other
                    // attachments remain (e.g. an explicit `flox deactivate`
                    // defers teardown to the executive). Remove any such
                    // start from state.json under the lock, then tear it down
                    // with the lock released: its hook.on-deactivate has no
                    // timeout and must not block new activations. The write
                    // retriggers StateFileChanged, which finds no orphans.
                    let orphaned = activations.remove_orphaned_starts();
                    if orphaned.is_empty() {
                        drop(lock);
                    } else {
                        write_activations_json(&activations, &state_json_path, lock)?;
                        sweep_orphaned_starts(
                            subsystem_verbosity,
                            &initial_attach_ctx,
                            &project_ctx,
                            &activation_state_dir,
                            orphaned,
                        );
                    }
                }
            },
            Ok(ExecutiveEvent::StateFileRemoved) => {
                // state.json existed when this executive started, so a removal
                // event means it was deleted at some point — typically by an
                // external actor such as a test harness or manual cleanup
                // removing the runtime dir. Either way this executive is done.
                //
                // The event only says state.json was missing at some point, so
                // take the lock and let cleanup_on_no_state decide under it.
                let lock = acquire_activations_json_lock(&state_json_path)
                    .context("can't cleanup after state file removal")?;
                return cleanup_on_no_state(
                    lock,
                    "handling a state.json removal",
                    &state_json_path,
                    &process_compose_bin,
                    &socket_path,
                    &activation_state_dir,
                );
            },
            Ok(ExecutiveEvent::SigChld) => {
                reap_orphaned_children();
            },
            Ok(ExecutiveEvent::TerminationSignal) => {
                // A termination signal (SIGINT/SIGTERM/SIGQUIT) is a normal exit for this
                // long-lived background process, not an error. We intentionally leave the
                // activation in the registry: we don't know who sent the signal or why, so
                // we can't safely run cleanup.
                info!(reason = "termination signal", "exiting without cleanup");
                return Ok(());
            },
            Err(_) => {
                bail!("event channel disconnected unexpectedly");
            },
        }
    }
}

/// Guards against infinite re-monitoring loops for the same PID.
/// If a PID is re-monitored `limit` times without being detached in between,
/// further re-monitoring is skipped.
struct LoopGuard {
    pid: Option<i32>,
    count: u32,
    limit: u32,
}

impl LoopGuard {
    fn new(limit: u32) -> Self {
        Self {
            pid: None,
            count: 0,
            limit,
        }
    }

    /// Record a PID re-monitoring attempt. Returns true if allowed, false if blocked.
    fn allow_remonitor(&mut self, pid: i32) -> bool {
        if self.pid == Some(pid) {
            self.count += 1;
        } else {
            self.pid = Some(pid);
            self.count = 1;
        }
        self.count <= self.limit
    }

    /// Record that a PID is no longer attached, clearing the counter if it
    /// was tracking that PID.
    fn reset_on_detach(&mut self, pid: i32) {
        if self.pid == Some(pid) {
            self.pid = None;
            self.count = 0;
        }
    }
}

/// Handle a process exit event by cleaning up state and determining if the loop should continue.
///
/// Returns `true` if all PIDs have terminated and cleanup completed (exit the loop),
/// or `false` if there are still active PIDs (continue the loop).
#[allow(clippy::too_many_arguments)]
fn handle_process_exited(
    pid: i32,
    coordinator: &EventCoordinator,
    loop_guard: &mut LoopGuard,
    state_json_path: &Path,
    process_compose_bin: &Path,
    socket_path: &Path,
    activation_state_dir: &Path,
    subsystem_verbosity: u32,
    initial_attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
) -> Result<bool> {
    // Remove from known_pids first so it can be re-monitored if it re-attached
    coordinator.stop_monitoring(pid);

    // Read and lock here rather than inside cleanup_pid: every arm of the event
    // loop answers "is there still activation state?" the same way, and keeping
    // that decision in one layer means cleanup_pid can stay a function over
    // state that is already known to exist.
    let (activations_json, lock) = read_activations_json(state_json_path)?;
    let Some(activations) = activations_json else {
        cleanup_on_no_state(
            lock,
            "cleaning up an exited PID",
            state_json_path,
            process_compose_bin,
            socket_path,
            activation_state_dir,
        )?;
        return Ok(true);
    };

    // Use PidWatcher to clean up the state
    match watcher::cleanup_pid((activations, lock), state_json_path, pid) {
        Ok(watcher::CleanupPidResult::Remaining { orphaned }) => {
            // Still have active PIDs - check if this PID re-attached
            // and needs to be monitored again.
            //
            // Note: This is intentionally redundant with the file watcher
            // in start_state_watcher(). The file watcher handles the normal
            // case where state.json is modified and we detect new PIDs.
            // However, if the PID re-attaches between stop_monitoring() and
            // cleanup_pids(), and the file watcher event hasn't fired yet,
            // this check ensures we don't miss it. The redundancy is safe
            // because start_monitoring() is idempotent.
            //
            // This also hypothetically catches the case where a PID exits
            // before its expiration and the watcher thread sends an event early.
            // That's not currently reachable because the watcher should sleep,
            // but the watcher shouldn't be treated as the authority on expired
            // PIDs.
            let (activations_json, lock) = read_activations_json(state_json_path)?;
            let Some(activations) = activations_json else {
                cleanup_on_no_state(
                    lock,
                    "checking for re-attached PIDs",
                    state_json_path,
                    process_compose_bin,
                    socket_path,
                    activation_state_dir,
                )?;
                return Ok(true);
            };
            // Check if the PID that triggered this event is still in state
            let pid_reused = activations
                .all_attached_pids_and_expiration()
                .into_iter()
                .find(|(attached_pid, _)| *attached_pid == pid);
            if let Some((pid, expiration)) = pid_reused {
                if loop_guard.allow_remonitor(pid) {
                    // info so that Sentry breadcrumbs show every iteration
                    // leading up to the loop guard tripping.
                    // The status is included because read_pid_status reports
                    // Dead both for a process that has exited and for one
                    // whose status couldn't be read, and re-monitoring a PID
                    // that doesn't report Running is what a loop looks like.
                    info!(
                        pid,
                        ?expiration,
                        status = ?read_pid_status(pid),
                        count = loop_guard.count,
                        "PID re-attached to activation, starting new monitor"
                    );
                    coordinator
                        .start_monitoring(pid, expiration)
                        .context("failed to restart monitoring for re-attached PID")?;
                } else {
                    error!(
                        pid,
                        ?expiration,
                        status = ?read_pid_status(pid),
                        count = loop_guard.count,
                        "PID re-monitored too many times, skipping to prevent loop"
                    );
                }
            } else {
                loop_guard.reset_on_detach(pid);
            }

            // Double check that all attached PIDs are monitored.
            // We're seeing flakey tests that double checking seems to fix.
            // It's not entirely clear what scenario is causing the flakes, but
            // it may be related to a race caused by firing the prompt hook.
            //
            // Another possible explanation would be during activation,
            // state.json replaces the short-lived activation PID
            // with the user shell PID. The executive normally discovers the replacement
            // through StateFileChanged, but filesystem notifications can be missed or
            // coalesced.
            // After ProcessExited, reread the current attachments and ensure every
            // remaining PID is monitored. This makes filesystem notifications a fast
            // path instead of a correctness requirement and allows teardown after the
            // replacement shell exits.
            //
            // We haven't pinpointed exactly why that would have started happening.
            //
            // See https://flox-dev.slack.com/archives/C05P6A5J6U8/p1785269662236939?thread_ts=1785243629.173519&cid=C05P6A5J6U8
            //
            // The PID that triggered this event is excluded because it was
            // just handled above. start_monitoring() would skip it anyway when
            // it was re-monitored, but not when the loop guard blocked it.
            let other_attached_pids = activations
                .all_attached_pids_and_expiration()
                .into_iter()
                .filter(|(attached_pid, _)| *attached_pid != pid)
                .collect();
            coordinator
                .ensure_monitoring_pids(other_attached_pids)
                .context("failed to ensure monitoring PIDs")?;
            drop(lock);

            // The exited PID may have been the last attachment for its start
            // even though other starts still have attachments. cleanup_pid
            // removed any such start from state.json under its lock; tear
            // them down now that no lock is held, because their
            // hook.on-deactivate has no timeout and must not block new
            // activations.
            sweep_orphaned_starts(
                subsystem_verbosity,
                initial_attach_ctx,
                project_ctx,
                activation_state_dir,
                orphaned,
            );

            Ok(false)
        },
        Ok(watcher::CleanupPidResult::AllDetached(locked_activations)) => {
            info!("running cleanup after all PIDs terminated");
            let cleaned_up = cleanup_all(
                locked_activations,
                process_compose_bin,
                socket_path,
                activation_state_dir,
                subsystem_verbosity,
                initial_attach_ctx,
                project_ctx,
            )
            .context("cleanup failed")?;
            Ok(cleaned_up)
        },
        Err(err) => {
            info!(%err, "running cleanup after error");
            let (activations_json, lock) = read_activations_json(state_json_path)?;
            let Some(activations) = activations_json else {
                // The original error is carried into the log rather than
                // returned: with the state gone there is nothing left to
                // recover, so exiting cleanly beats a failure the executive
                // cannot act on.
                cleanup_on_no_state(
                    lock,
                    &format!("cleaning up after error: {err}"),
                    state_json_path,
                    process_compose_bin,
                    socket_path,
                    activation_state_dir,
                )?;
                return Ok(true);
            };
            let _ = cleanup_all(
                (activations, lock),
                process_compose_bin,
                socket_path,
                activation_state_dir,
                subsystem_verbosity,
                initial_attach_ctx,
                project_ctx,
            );
            bail!(err.context("failed while waiting for termination"))
        },
    }
}

/// Handle the SIGUSR1 signal to start process-compose.
///
/// Return:
/// - `Some(LockedActivationState)` if state was modified and needs to be written
/// - `None` if there were no changes and the lock was dropped
fn handle_start_services_signal(
    locked_activations: LockedActivationState,
    subsystem_verbosity: u32,
    attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
    activation_state_dir: &Path,
) -> Result<Option<LockedActivationState>> {
    let (mut activations, lock) = locked_activations;

    // There's nothing we can do if another "start" has occurred in the time it
    // took us to receive and process the signal. `flox-activations activate`
    // may timeout and present an error to the user.
    let Some(ready_start_id) = activations.ready_start_id().cloned() else {
        info!(
            reason = "no currently ready activation to attach",
            "skipping process-compose start"
        );
        return Ok(None);
    };

    // `flox-activations activate` ensures that `process-compose` is stopped
    // (and the socket removed) before signaling a restart.
    if project_ctx.flox_services_socket.exists() {
        info!(reason = "already running", "skipping process-compose start");
        return Ok(None);
    }

    start_process_compose_no_services(
        subsystem_verbosity,
        attach_ctx,
        project_ctx,
        &ready_start_id,
        activation_state_dir,
    )?;

    activations.set_current_process_compose_start_id(ready_start_id);

    Ok(Some((activations, lock)))
}

/// Shut down what can still be reached once state.json is gone.
///
/// Without state.json there is no attachment list to consult and no state left
/// to remove, so stopping `process-compose` if its socket outlived the state is
/// the only useful thing remaining. Attached processes may outlive the state;
/// the executive can do nothing further for them.
///
/// Takes the lock rather than a path so that "state.json is absent" is decided
/// while holding it. state.json cannot be written without the lock, so that
/// answer cannot change underneath us — unlike a bare `exists()`, which is only
/// ever a statement about the past.
///
/// `discovered_during` names the operation that ran into the missing state.
/// Several unrelated paths converge here, and which one noticed is the useful
/// thing to know when reading an executive log, so it is carried into both
/// outcomes.
///
/// Returns an error when the state really is gone: an activation being
/// destroyed out from under a running executive is an anomaly worth a non-zero
/// exit, even though nothing here can recover from it. Cleanup still runs
/// first. Returns `Ok` only for the benign case where a new activation has
/// taken over.
fn cleanup_on_no_state(
    _hold_the_lock: LockFile,
    discovered_during: &str,
    state_json_path: &Path,
    process_compose_bin: &Path,
    socket_path: &Path,
    activation_state_dir_path: &Path,
) -> Result<()> {
    // If state.json is back, it can only have been recreated by a new `start`,
    // whose executive now owns the state and any services — leave both alone.
    // That is a handoff rather than a failure.
    if state_json_path.exists() {
        info!(
            discovered_during,
            reason = "state.json recreated by a new activation",
            "exiting without cleanup"
        );
        return Ok(());
    }

    // Acquiring the lock recreates the state dir when an external `rm -rf` took
    // it, so removing it here is what keeps this from leaving a directory and a
    // stale state.lock behind.
    shut_down_and_remove_state(process_compose_bin, socket_path, activation_state_dir_path)
        .context("failed to clean up after removed activation state")?;

    bail!("activation state was removed while {discovered_during}")
}

/// Shutdown `process-compose` if running and remove the activation state
/// directory.
///
/// Used when state.json is gone and the starts on disk can't be trusted;
/// no `hook.on-deactivate` runs on this path.
fn shut_down_and_remove_state(
    process_compose_bin: &Path,
    socket_path: impl AsRef<Path>,
    activation_state_dir_path: impl AsRef<Path>,
) -> Result<()> {
    shut_down_process_compose(process_compose_bin, socket_path.as_ref());
    let cleanup_path = rename_state_for_removal(activation_state_dir_path.as_ref())?;
    fs::remove_dir_all(&cleanup_path).context("couldn't remove activations dir")?;
    Ok(())
}

/// Shutdown `process-compose` if its socket is present.
fn shut_down_process_compose(process_compose_bin: &Path, socket_path: &Path) {
    if !socket_path.exists() {
        info!(reason = "no socket", "did not shut down process-compose");
    } else if let Err(err) = process_compose_down(process_compose_bin, socket_path) {
        warn!(%err, "failed to run process-compose shutdown command");
    } else {
        info!("shut down process-compose");
    }
}

/// Atomically detach the activation state directory for removal by renaming
/// it, returning the renamed path.
///
/// We want to avoid a race where remove_dir_all removes the lock before
/// removing activation state dir,
/// and then another activation creates a lock and causes remove_dir_all to
/// fail.
fn rename_state_for_removal(activation_state_dir_path: &Path) -> Result<PathBuf> {
    let cleanup_path =
        activation_state_dir_path.with_extension(format!("cleanup.{}", std::process::id()));
    fs::rename(activation_state_dir_path, &cleanup_path)
        .context("couldn't rename activations dir for cleanup")?;
    Ok(cleanup_path)
}

/// Shutdown `process-compose` if running, run any `hook.on-deactivate`
/// bookends, and remove all activation state.
/// To be called when there are no longer any PIDs attached.
/// Returns `true` if cleanup ran, `false` if PIDs were found and cleanup was skipped.
#[allow(clippy::too_many_arguments)]
fn cleanup_all(
    locked_activations: LockedActivationState,
    process_compose_bin: &Path,
    socket_path: impl AsRef<Path>,
    activation_state_dir_path: impl AsRef<Path>,
    subsystem_verbosity: u32,
    initial_attach_ctx: &AttachCtx,
    project_ctx: &AttachProjectCtx,
) -> Result<bool> {
    info!("running cleanup");

    let (mut activations_json, hold_the_lock) = locked_activations;

    if !activations_json.attached_pids_is_empty() {
        warn!("cleanup called with PIDs still attached, skipping");
        return Ok(false);
    }

    shut_down_process_compose(process_compose_bin, socket_path.as_ref());
    let cleanup_path = rename_state_for_removal(activation_state_dir_path.as_ref())?;
    // The rename already detached the state from new activations (they
    // recreate the directory under its original name), so the lock guards
    // nothing anymore; release it before running hooks.
    drop(hold_the_lock);

    // Run hook.on-deactivate for the remaining start(s), mirroring the
    // activation order in reverse: services were shut down above, the hook
    // bookends close last. With no attachments left every start in the state
    // is orphaned, including starts deferred by an explicit detach that the
    // executive never got to sweep. No state.json write is needed — the whole
    // directory is removed below.
    let orphaned = activations_json.remove_orphaned_starts();
    sweep_orphaned_starts(
        subsystem_verbosity,
        initial_attach_ctx,
        project_ctx,
        &cleanup_path,
        orphaned,
    );

    fs::remove_dir_all(&cleanup_path).context("couldn't remove activations dir")?;

    info!("finished cleanup");

    Ok(true)
}

#[cfg(test)]
mod test {
    use std::collections::BTreeMap;

    use flox_core::activate::mode::ActivateMode;
    use flox_core::activations::test_helpers::{read_activation_state, write_activation_state};
    use flox_core::activations::{ActivationState, StartOrAttachResult, activation_state_dir_path};

    use super::event_coordinator::{EventCoordinator, ExecutiveEvent};
    use super::watcher::test::{start_process, stop_process, test_context};
    use super::*;

    #[test]
    fn monitoring_loop_removes_state_on_cleanup() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Create an ActivationState with one PID attached
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);

        // Write state to disk
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        assert!(
            activation_state_directory.exists(),
            "state directory should exist before cleanup"
        );

        // Create a coordinator for testing - the loop will use pid watchers to detect exit
        let coordinator = EventCoordinator::new().unwrap();
        let state = read_activation_state(runtime_dir, &dot_flox_path);
        coordinator
            .ensure_monitoring_pids(state.all_attached_pids_and_expiration())
            .unwrap();

        stop_process(proc);

        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        run_event_loop(
            attach,
            project,
            activation_state_directory.clone(),
            coordinator,
            0,
        )
        .unwrap();

        // Verify state directory is completely removed after cleanup
        assert!(
            !activation_state_directory.exists(),
            "state directory should be removed after cleanup"
        );
    }

    #[test]
    fn monitoring_loop_exits_without_cleanup_on_termination_signal() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        let proc = start_process();
        let pid = proc.id() as i32;

        // Create an ActivationState with one PID attached
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);

        // Write state to disk
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        assert!(
            activation_state_directory.exists(),
            "state directory should exist before monitoring loop"
        );

        // Create coordinator and inject termination event before starting the loop
        let coordinator = EventCoordinator::new().unwrap();
        let state = read_activation_state(runtime_dir, &dot_flox_path);
        coordinator
            .ensure_monitoring_pids(state.all_attached_pids_and_expiration())
            .unwrap();
        coordinator.inject_event(ExecutiveEvent::TerminationSignal);

        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        let result = run_event_loop(
            attach,
            project,
            activation_state_directory.clone(),
            coordinator,
            0,
        );

        // Verify the loop exited normally (a termination signal is not an error)
        result.expect("termination signal should exit cleanly without error");

        // Verify cleanup did NOT occur - state directory should still exist
        assert!(
            activation_state_directory.exists(),
            "state directory should NOT be removed when exiting due to termination signal"
        );

        // Clean up the process
        stop_process(proc);
    }

    /// Test that handle_process_exited monitors replacement PIDs even when no
    /// StateFileChanged event is delivered.
    ///
    /// This can happen in two cases:
    /// 1. --remove-pid removes a PID from state.json, but the
    ///    executive still has a watcher running for that PID.
    /// 2. `flox-activations detach` removes a PID from state.json
    ///
    /// In either case, handling the stale ProcessExited event must reconcile
    /// the replacement PID directly. Filesystem notifications are a fast path,
    /// not the only path that preserves cleanup correctness.
    #[test]
    fn process_exited_reconciles_replacement_pid() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // Start two processes - proc1 will be replaced by proc2
        let proc1 = start_process();
        let pid1 = proc1.id() as i32;
        let proc2 = start_process();
        let pid2 = proc2.id() as i32;

        // Create an ActivationState with pid1 attached
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);

        // Replace pid1 with pid2 (simulating --remove-pid which uses replace_attachment)
        state
            .replace_attachment(start_id.clone(), pid1, pid2, None)
            .unwrap();

        // Write state to disk (now only has pid2)
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        let state_json = state_json_path(&activation_state_directory);

        // Re-read state to compare later
        let initial_state = read_activation_state(runtime_dir, &dot_flox_path);
        let attachments = initial_state.attachments_by_start_id();
        assert_eq!(
            attachments,
            BTreeMap::from([(start_id.clone(), vec![(pid2, None)])]),
            "initial state should have pid2 attached to start_id"
        );

        // These are all dummy values
        let coordinator = EventCoordinator::new().unwrap();
        let mut loop_guard = LoopGuard::new(5);
        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        stop_process(proc1);

        // Call handle_process_exited for pid1 (which is not in state.json)
        let result = handle_process_exited(
            pid1,
            &coordinator,
            &mut loop_guard,
            &state_json,
            &project.process_compose_bin,
            &project.flox_services_socket,
            &activation_state_directory,
            0,
            &attach,
            &project,
        );

        // No cleanup is needed while the replacement PID is active.
        assert!(
            matches!(result, Ok(false)),
            "handle_process_exited should keep monitoring while replacement PIDs remain, got: {:?}",
            result
        );

        // Re-read state and verify it hasn't changed
        let final_state = read_activation_state(runtime_dir, &dot_flox_path);
        assert_eq!(initial_state, final_state);

        // Show that pid2 is monitored by stopping it and asserting we receive
        // its exit event. No StateFileChanged event is ever injected here, so
        // only handle_process_exited could have started that watcher.
        stop_process(proc2);
        let event = coordinator
            .receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("replacement PID should be monitored");
        assert_eq!(event, ExecutiveEvent::ProcessExited { pid: pid2 });

        let result = handle_process_exited(
            pid2,
            &coordinator,
            &mut loop_guard,
            &state_json,
            &project.process_compose_bin,
            &project.flox_services_socket,
            &activation_state_directory,
            0,
            &attach,
            &project,
        );
        assert!(
            matches!(result, Ok(true)),
            "replacement PID exit should clean up the activation, got: {result:?}"
        );
        assert!(
            !activation_state_directory.exists(),
            "activation state should be removed after the replacement PID exits"
        );
    }

    #[test]
    fn loop_guard_blocks_after_limit() {
        let mut guard = LoopGuard::new(2);
        let pid = 12345;

        // First two calls should be allowed
        assert!(guard.allow_remonitor(pid));
        assert!(guard.allow_remonitor(pid));

        assert!(!guard.allow_remonitor(pid), "third call should be blocked");

        // Different PID resets the counter
        let other_pid = 67890;
        assert!(guard.allow_remonitor(other_pid));
        assert!(
            guard.allow_remonitor(pid),
            "counter should reset after different PID"
        );
    }

    #[test]
    fn loop_guard_resets_when_tracked_pid_detaches() {
        let mut guard = LoopGuard::new(2);
        let pid = 12345;

        assert!(guard.allow_remonitor(pid));
        assert!(guard.allow_remonitor(pid));
        assert!(!guard.allow_remonitor(pid));

        // Detaching a different PID doesn't reset the counter
        guard.reset_on_detach(67890);
        assert!(!guard.allow_remonitor(pid));

        guard.reset_on_detach(pid);
        assert!(
            guard.allow_remonitor(pid),
            "counter should reset after the PID detached"
        );
    }

    /// A PID exit that resolves in a detach rather than a re-attach resets
    /// the loop guard so later legitimate re-attaches of the same PID aren't
    /// blocked.
    #[test]
    fn handle_process_exited_resets_loop_guard_on_detach() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // One PID that will exit and one that keeps the activation alive
        let exited = start_process();
        let exited_pid = exited.id() as i32;
        let keeper = start_process();
        let keeper_pid = keeper.id() as i32;

        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(exited_pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        let result = state.start_or_attach(keeper_pid, &store_path);
        let StartOrAttachResult::Attach { .. } = result else {
            panic!("Expected Attach")
        };
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        let state_json = state_json_path(&activation_state_directory);

        let coordinator = EventCoordinator::new().unwrap();
        let mut loop_guard = LoopGuard::new(2);
        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        // Exhaust the guard for the PID
        assert!(loop_guard.allow_remonitor(exited_pid));
        assert!(loop_guard.allow_remonitor(exited_pid));
        assert!(!loop_guard.allow_remonitor(exited_pid));

        stop_process(exited);

        // The exited PID is detached, which resets the guard
        let result = handle_process_exited(
            exited_pid,
            &coordinator,
            &mut loop_guard,
            &state_json,
            &project.process_compose_bin,
            &project.flox_services_socket,
            &activation_state_directory,
            0,
            &attach,
            &project,
        );
        assert!(matches!(result, Ok(false)), "keeper PID should remain");

        assert!(
            loop_guard.allow_remonitor(exited_pid),
            "guard should reset after the PID detached"
        );

        stop_process(keeper);
    }

    /// Test that handle_process_exited increments the loop guard when a PID
    /// is still in state.json and needs to be re-monitored.
    ///
    /// This simulates the edge case where a PID exits but is immediately
    /// re-used by a new attachment before cleanup completes.
    #[test]
    fn handle_process_exited_increments_loop_guard() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // Start a process that will stay running (so cleanup_pid won't remove it)
        let proc = start_process();
        let pid = proc.id() as i32;

        // Create state with this PID attached
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(pid, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        let state_json = state_json_path(&activation_state_directory);

        let coordinator = EventCoordinator::new().unwrap();
        // Use a low limit so we can test hitting it
        let mut loop_guard = LoopGuard::new(2);
        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        // First call: PID is in state and running, so it will be re-monitored.
        // loop_guard.allow_remonitor(pid) returns true (count=1 < limit=2)
        let result = handle_process_exited(
            pid,
            &coordinator,
            &mut loop_guard,
            &state_json,
            &project.process_compose_bin,
            &project.flox_services_socket,
            &activation_state_directory,
            0,
            &attach,
            &project,
        );
        assert!(matches!(result, Ok(false)), "first call should succeed");

        // Second call: loop_guard.allow_remonitor(pid) returns false (count=2 >= limit=2)
        // The re-monitoring should be skipped
        let result = handle_process_exited(
            pid,
            &coordinator,
            &mut loop_guard,
            &state_json,
            &project.process_compose_bin,
            &project.flox_services_socket,
            &activation_state_directory,
            0,
            &attach,
            &project,
        );
        assert!(matches!(result, Ok(false)), "second call should succeed");

        // Verify the guard state: calling allow_remonitor again should return false
        // since we've hit the limit
        assert!(
            !loop_guard.allow_remonitor(pid),
            "loop guard should block after handle_process_exited incremented it"
        );

        stop_process(proc);
    }

    /// When state.json is updated such that all PIDs are gone (e.g., by
    /// `flox-activations detach` for an in-place deactivation where the shell
    /// process is still alive), the monitoring loop should trigger cleanup
    /// immediately via StateFileChanged rather than waiting for ProcessExited.
    #[test]
    fn state_file_change_triggers_cleanup_when_no_pids() {
        let temp_dir = tempfile::tempdir().unwrap();
        let runtime_dir = temp_dir.path();
        let dot_flox_path = PathBuf::from(".flox");
        let flox_env = dot_flox_path.join("run/test");
        let store_path = "store_path".to_string();

        // Create activation state with a PID but then write it with 0 PIDs
        // (simulating what happens after `flox-activations detach` removes
        // the last PID while the process is still running).
        let mut state =
            ActivationState::new(&ActivateMode::default(), Some(&dot_flox_path), &flox_env);
        let result = state.start_or_attach(1, &store_path);
        let StartOrAttachResult::Start { start_id, .. } = result else {
            panic!("Expected Start")
        };
        state.set_ready(&start_id);
        // Detach the PID in-memory so the written state has 0 attached PIDs.
        let _ = state.detach(1);
        write_activation_state(runtime_dir, &dot_flox_path, state);

        let activation_state_directory = activation_state_dir_path(runtime_dir, &dot_flox_path);
        assert!(
            activation_state_directory.exists(),
            "state directory should exist before the event loop"
        );

        // Inject a StateFileChanged event so the loop reacts as if the file
        // had just been written by `detach`.
        let coordinator = EventCoordinator::new().unwrap();
        coordinator.inject_event(ExecutiveEvent::StateFileChanged);

        let (attach, project) = test_context(&dot_flox_path, &flox_env.to_string_lossy());

        run_event_loop(
            attach,
            project,
            activation_state_directory.clone(),
            coordinator,
            0,
        )
        .expect("event loop should exit cleanly after cleanup");

        assert!(
            !activation_state_directory.exists(),
            "state directory should be removed after cleanup via StateFileChanged"
        );
    }
}
