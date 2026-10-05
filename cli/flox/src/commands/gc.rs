//! # Garbage collection
//!
//! Garbage collection happens in two phases:
//! - Cleaning the environment registry
//! - Cleaning the Nix store
//!
//! ## Environment registry
//!
//! We clean the environment registry first because it creates GC roots.
//! See the environment registry module for more details on how we prune
//! stale environments from the registry.
//!
//! ## Nix store
//!
//! To clean the Nix store we run `nix store gc` and parse its output to
//! display a spinner. We do this because the GC process can take a long time
//! and we don't want users to thing that the CLI is broken.
//!
//! By inspecting the output of `nix store gc` you can see that it follows a
//! process, but there's a lot of extraneous information in the output that a
//! user doesn't need to care about. To keep the signal to noise ratio high for
//! the user, we create our own state machine from the output of `nix store gc`.
//!
//! Our state machine for garbage collection of the Nix store is shown below.
//! Depending on the state of the user's store, certain steps might be skipped.
//!
//!      ┌────┐                                   
//!      │Init│                                   
//!      └─┬──┘                                   
//!        │                                      
//!        ▼                                      
//!      ┌───────┐                                
//!      │Finding│                                
//! ┌─┬──┼ Roots │                                
//! │ │  └─┬─────┘                                
//! │ │    │                                      
//! │ │    ▼                                      
//! │ │  ┌────────┐                               
//! │ │  │Removing┼────┐                          
//! │ │  │  Links │    │                          
//! │ │  └─┬──────┘    │                          
//! │ │    │           │                          
//! │ │    ▼           │                          
//! │ │  ┌──────────┐  │                          
//! │ └─►│ Scanning ┼──┼─┐                        
//! │    └─┬────────┘  │ │                        
//! │      │           │ │                        
//! │      ▼           │ │ Skip straight to "done"
//! │    ┌────────┐    │ │ if there was no garbage
//! └───►│Deleting│◄───┘ │                        
//!      └─┬──────┘      │                        
//!        │             │                        
//!        ▼             │                        
//!      ┌────┐          │                        
//!      │Done│ ◄────────┘                        
//!      └────┘                                        
//!
//! Note that `nix store gc` doesn't report any progress unless you specify
//! the `--debug` or `-vv` flags. It also reports progress to `stderr` whereas
//! the final amount of freed disk space is reported to `stdout`.

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, ChildStderr, ChildStdout, Stdio};

use anyhow::{Context, Result, anyhow};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::env_registry;
use flox_rust_sdk::providers::nix::nix_base_command;
use tracing::{Span, debug, info_span, instrument, trace};

use crate::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Gc {}

impl Gc {
    #[instrument(skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let span = info_span!("collecting_garbage", progress = "Collecting garbage");
        let _guard = span.enter();
        env_registry::garbage_collect(&flox)?;
        let freed = run_store_gc()?;
        drop(_guard);
        message::info(freed);
        message::updated("Garbage collection complete");
        Ok(())
    }
}

/// Represents the stages of garbage collection in the logs of the
/// `nix store gc` command.
#[derive(Debug, Clone, PartialEq, Eq)]
enum GcProgress {
    /// The initial state, which is very short lived.
    Init,
    /// Finding garbage collector roots.
    FindingRoots,
    /// If any stale links are found, this stage deletes them. If no stale links
    /// are found, this stage may be skipped.
    RemovingLinks,
    /// Scanning store paths for liveness.
    Scanning,
    /// Deleting the stale store paths. Contains a message indicating which
    /// store path is being deleted.
    DeletingStorePaths(String),
}

impl GcProgress {
    fn new() -> Self {
        GcProgress::Init
    }

    fn msg(&self) -> Option<String> {
        use GcProgress::*;
        match self {
            Init => None,
            FindingRoots => Some("Finding garbage collector roots".to_string()),
            RemovingLinks => Some("Removing stale garbage collector roots".to_string()),
            Scanning => Some("Scanning packages for liveness".to_string()),
            DeletingStorePaths(msg) => Some(msg.clone()),
        }
    }

    fn state_name(&self) -> &'static str {
        use GcProgress::*;
        match self {
            Init => "init",
            FindingRoots => "finding_roots",
            RemovingLinks => "removing_links",
            Scanning => "scanning",
            DeletingStorePaths(_) => "deleting",
        }
    }

    fn is_removing_links_line(line: &str) -> bool {
        line.starts_with("removing stale link")
    }

    fn is_finding_roots_line(line: &str) -> bool {
        line.starts_with("finding garbage collector roots")
    }

    fn is_scanning_line(line: &str) -> bool {
        line.starts_with("cannot delete")
    }

    fn is_deletion_line(line: &str) -> bool {
        line.starts_with("deleting '/nix/store")
    }

    fn new_progress_state(&self, line: &str) -> Option<GcProgress> {
        use GcProgress::*;
        match self {
            Init => {
                if GcProgress::is_finding_roots_line(line) {
                    Some(GcProgress::FindingRoots)
                } else {
                    None
                }
            },
            FindingRoots => {
                if GcProgress::is_removing_links_line(line) {
                    Some(GcProgress::RemovingLinks)
                } else if GcProgress::is_scanning_line(line) {
                    Some(GcProgress::Scanning)
                } else if GcProgress::is_deletion_line(line) {
                    //  There may not be stale links, so we should detect
                    //  the 'deleting...' line as well and possibly skip
                    //  this stage if necessary
                    let msg = Self::capitalize_deleting_store_path_line(line);
                    Some(GcProgress::DeletingStorePaths(msg))
                } else {
                    None
                }
            },
            RemovingLinks => {
                if GcProgress::is_deletion_line(line) {
                    // Need to capitalize the 'd' so it matches the case of
                    // the other log messages.
                    let msg = Self::capitalize_deleting_store_path_line(line);
                    Some(GcProgress::DeletingStorePaths(msg))
                } else if GcProgress::is_scanning_line(line) {
                    Some(GcProgress::Scanning)
                } else {
                    None
                }
            },
            Scanning => {
                if GcProgress::is_deletion_line(line) {
                    // Need to capitalize the 'd' so it matches the case of
                    // the other log messages.
                    let msg = Self::capitalize_deleting_store_path_line(line);
                    Some(GcProgress::DeletingStorePaths(msg))
                } else {
                    None
                }
            },
            DeletingStorePaths(_msg) => {
                if GcProgress::is_deletion_line(line) {
                    let msg = Self::capitalize_deleting_store_path_line(line);
                    Some(GcProgress::DeletingStorePaths(msg))
                } else {
                    None
                }
            },
        }
    }

    fn capitalize_deleting_store_path_line(line: &str) -> String {
        let mut msg = line.to_string();
        msg.replace_range(0..1, "D");
        msg
    }
}

fn gc_command() -> std::process::Command {
    let mut cmd = nix_base_command();
    // The `--debug` is intentional here. You don't get any progress info
    // without it.
    cmd.args(["store", "gc", "--debug"]);
    cmd.stderr(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd
}

/// Get handles to stderr and stdout of the GC process. The progress is reported on
/// stderr, but the final result is reported on stdout.
fn gc_readers(
    cmd: &mut std::process::Command,
) -> Result<(ChildStdout, BufReader<ChildStderr>, Child)> {
    let mut proc = cmd.spawn().context("Failed to start GC process")?;
    let stderr = proc
        .stderr
        .take()
        .ok_or(anyhow!("Failed to get stderr from GC process"))?;
    debug!("spawned gc process");
    let reader = BufReader::new(stderr);
    let stdout = proc
        .stdout
        .take()
        .ok_or(anyhow!("Failed to get stdout from GC process"))?;
    Ok((stdout, reader, proc))
}

/// Runs `nix store gc` and updates the spinner so that there's an indication
/// that garbage collection hasn't stalled.
#[instrument(skip_all, fields(progress = "Garbage collecting package data"))]
fn run_store_gc() -> Result<String> {
    let mut cmd = gc_command();
    let (mut stdout, reader, mut proc) = gc_readers(&mut cmd)?;
    let (sender, receiver) = std::sync::mpsc::channel::<GcProgress>();

    let mut gc_progress = GcProgress::new();
    // It's necessary to keep track of the parent span (the one from the `instrument`
    // macro) so that we can associate the spans in the spinner thread with it.
    // Otherwise, you get a spooky spinner message.
    let parent_span = Span::current();

    // Spawn a separate background thread for reading and parsing the output of the
    // `nix store gc` command.
    let freed_msg = std::thread::scope(move |s| {
        // This thread collects and parses logs from the output of the `nix store gc`
        // command. It keeps track of the GC state so that we can inform the
        // spinner thread only when we need to.
        let reader_thread: std::thread::ScopedJoinHandle<
            '_,
            std::result::Result<(), anyhow::Error>,
        > = s.spawn(move || {
            for line in reader.lines() {
                let line = line.context("Failed to read GC process output")?;
                trace!(line, "line from gc reader");
                if let Some(new_progress) = gc_progress.new_progress_state(&line) {
                    debug!(line, "got new progress state");
                    sender
                        .send(new_progress.clone())
                        .context("Background thread exited early")?;
                    gc_progress = new_progress;
                }
            }
            debug!("consumed all output, exiting gc reader thread");
            Ok(())
        });
        debug!("spawned gc reader thread");

        // Manage the spinner state on the current thread.
        let mut span_guard = Some(
            info_span!(parent: &parent_span, "gc_progress", progress = "Initializing").entered(),
        );
        // We'll get an error once the sender hangs up.
        while let Ok(new_progress) = receiver.recv() {
            // This dance is to make sure that the span (and hence the spinner)
            // stays open until the _start of the next loop body_,
            // rather than closing at the end of the current loop body
            // (which is immediately, basically).
            if let Some(guard) = span_guard.take() {
                drop(guard);
            }
            span_guard = Some(
                info_span!(
                    parent: &parent_span,
                    "gc_progress",
                    progress = new_progress.msg().unwrap_or("Finishing".to_string())
                )
                .entered(),
            );
            debug!(
                state = new_progress.state_name(),
                "updated spinner with new state"
            );
        }
        drop(span_guard.take());
        debug!("gc log sender hung up");

        let freed_msg = {
            let mut buf = String::new();
            stdout
                .read_to_string(&mut buf)
                .context("failed to read GC process stdout")?;
            buf.trim().to_string()
        };
        // Wait for the reader thread to exit.
        reader_thread
            .join()
            .map_err(|_| anyhow!("Background thread panicked"))??;
        proc.wait()
            .context("Failed while waiting for GC process to exit")?;
        debug!("joined gc reader thread");
        Ok::<String, anyhow::Error>(freed_msg)
    })?;
    Ok(freed_msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_sequence(lines: &[&str]) -> Vec<GcProgress> {
        let mut gc_progress = GcProgress::new();
        let mut states = vec![gc_progress.clone()];
        for line in lines.iter() {
            if let Some(progress) = gc_progress.new_progress_state(line) {
                states.push(progress.clone());
                gc_progress = progress;
            }
        }
        states
    }

    #[test]
    fn ingests_full_sequence() {
        let lines = vec![
            "dummy_line",
            "finding garbage collector roots",
            "dummy_line",
            "removing stale link '/nix/store/abcdefg'",
            "dummy_line",
            "cannot delete '/nix/store/abcdefg' because it's a root",
            "dummy_line", // This line shouldn't trigger a transition
            "cannot delete '/nix/store/abcdefg' because it's a root",
            "dummy_line",
            "deleting '/nix/store/abcdefg/'",
            "dummy_line", // This line shouldn't trigger a transition
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "dummy_line",
        ];

        let states = state_sequence(&lines);
        assert!(!states.is_empty());
        let mut states = states.into_iter();
        assert!(matches!(states.next().unwrap(), GcProgress::Init));
        assert!(matches!(states.next().unwrap(), GcProgress::FindingRoots));
        assert!(matches!(states.next().unwrap(), GcProgress::RemovingLinks));
        assert!(matches!(states.next().unwrap(), GcProgress::Scanning));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(states.next().is_none());
    }

    #[test]
    fn skips_removing_links_when_not_present() {
        let lines = vec![
            "dummy_line",
            "finding garbage collector roots",
            "dummy_line", // "removing stale link" would normally appear here
            "cannot delete '/nix/store/abcdefg' because it's a root",
            "dummy_line", // dummy line here shouldn't trigger a transition
            "cannot delete '/nix/store/abcdefg' because it's a root",
            "dummy_line",
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "dummy_line",
        ];

        let states = state_sequence(&lines);
        assert!(!states.is_empty());
        let mut states = states.into_iter();
        assert!(matches!(states.next().unwrap(), GcProgress::Init));
        assert!(matches!(states.next().unwrap(), GcProgress::FindingRoots));
        assert!(matches!(states.next().unwrap(), GcProgress::Scanning));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(states.next().is_none());
    }

    #[test]
    fn skips_scanning_when_not_present() {
        let lines = vec![
            "dummy_line",
            "finding garbage collector roots",
            "dummy_line",
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "deleting '/nix/store/abcdefg/'",
            "dummy_line",
        ];

        let states = state_sequence(&lines);
        assert!(!states.is_empty());
        let mut states = states.into_iter();
        assert!(matches!(states.next().unwrap(), GcProgress::Init));
        assert!(matches!(states.next().unwrap(), GcProgress::FindingRoots));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(matches!(
            states.next().unwrap(),
            GcProgress::DeletingStorePaths(_)
        ));
        assert!(states.next().is_none());
    }
}
