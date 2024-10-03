use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context};
use flox_rust_sdk::models::env_registry::{activation_pids, ActivationPid};
use kqueue::Ident;
use tracing::warn;

use super::{Error, WaitResult, Watcher, WATCHER_SLEEP_INTERVAL};

/// Stores a list of PIDs to monitor via kqueue and waits for them to terminate.
#[derive(Debug)]
pub struct MacOsWatcher {
    pub original_pid: ActivationPid,
    pub pids_watching: HashSet<ActivationPid>,
    pub reg_path: PathBuf,
    pub hash: String,
    pub should_terminate_flag: Arc<AtomicBool>,
    pub should_clean_up_flag: Arc<AtomicBool>,
    pub kqueue_watcher: Option<kqueue::Watcher>,
}

impl MacOsWatcher {
    pub fn new(
        pid: ActivationPid,
        reg_path: impl AsRef<Path>,
        hash: impl AsRef<str>,
        should_terminate_flag: Arc<AtomicBool>,
        should_clean_up_flag: Arc<AtomicBool>,
    ) -> Self {
        MacOsWatcher {
            original_pid: pid,
            pids_watching: HashSet::new(),
            reg_path: PathBuf::from(reg_path.as_ref()),
            hash: String::from(hash.as_ref()),
            should_terminate_flag,
            should_clean_up_flag,
            kqueue_watcher: None,
        }
    }

    /// Initialize the internal kqueue watcher and add the PID for the activation
    /// that spawned this watchdog.
    fn init_kqueue(&mut self) -> Result<(), Error> {
        let mut kw = kqueue::Watcher::new().context("failed to create kqueue watcher")?;
        kw.add_pid(
            self.original_pid.into(),
            kqueue::EventFilter::EVFILT_PROC,
            kqueue::FilterFlag::NOTE_EXIT,
        )?;
        self.pids_watching.insert(self.original_pid);
        kw.watch().context("failed to initialize kqueue watcher")?;
        self.kqueue_watcher = Some(kw);
        Ok(())
    }

    /// Add a PID to the kqueue watcher to get termination notifications.
    fn add_pid(&mut self, pid: ActivationPid) -> Result<(), Error> {
        if let Some(ref mut kw) = self.kqueue_watcher {
            kw.add_pid(
                pid.into(),
                kqueue::EventFilter::EVFILT_PROC,
                kqueue::FilterFlag::NOTE_EXIT,
            )
            .with_context(|| format!("failed to add PID {pid} to watch list"))
        } else {
            Err(anyhow!("tried to watch PID without initializing kqueue"))
        }
    }

    /// Prunes any activation PIDs that have terminated
    fn prune_terminations(&mut self) -> Result<(), Error> {
        if let Some(ref mut kw) = self.kqueue_watcher {
            while let Some(event) = kw.poll(None) {
                if let Ident::Pid(pid) = event.ident {
                    if !self.pids_watching.remove(&pid.into()) {
                        warn!("received notification for PID that wasn't watched");
                    }
                }
            }
        } else {
            bail!("tried to prune activations without initializing kqueue");
        }
        Ok(())
    }
}

impl Watcher for MacOsWatcher {
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error> {
        self.init_kqueue()?;
        loop {
            self.update_watchlist()?;
            if self.should_clean_up()? {
                return Ok(WaitResult::CleanUp);
            }
            if self
                .should_terminate_flag
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(WaitResult::Terminate);
            }
            if self
                .should_clean_up_flag
                .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Ok(WaitResult::CleanUp);
            }
            std::thread::sleep(WATCHER_SLEEP_INTERVAL);
        }
    }

    /// Update the list of PIDs that are currently being watched.
    fn update_watchlist(&mut self) -> Result<(), Error> {
        let all_registered_pids = activation_pids(&self.reg_path, &self.hash)?;
        self.prune_terminations()?;
        let to_add = self
            .pids_watching
            .difference(&all_registered_pids)
            .cloned()
            .collect::<Vec<_>>();
        for pid in to_add {
            if pid.is_running() {
                self.add_pid(pid)?;
                if self.pids_watching.insert(pid) {
                    bail!("tried to watch PID {pid}, which was already watched");
                }
            }
        }
        Ok(())
    }

    fn should_clean_up(&self) -> Result<bool, super::Error> {
        Ok(self.pids_watching.is_empty())
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use flox_rust_sdk::models::env_registry::register_activation;

    use super::*;
    use crate::process::test::{
        path_for_registry_with_entry,
        shutdown_flags,
        start_process,
        stop_process,
    };

    #[test]
    fn error_when_initialized_with_terminated_pid() {
        let dummy_proc = start_process();
        let pid = ActivationPid::from(dummy_proc.id() as i32);
        stop_process(dummy_proc);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let mut watcher = MacOsWatcher::new(pid, "", "", terminate_flag, cleanup_flag);
        assert!(watcher.init_kqueue().is_err());
    }

    #[test]
    fn terminates_when_all_pids_terminate() {
        let proc1 = start_process();
        let pid1 = ActivationPid::from(proc1.id() as i32);
        let proc2 = start_process();
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid1).unwrap();
        let mut watcher =
            MacOsWatcher::new(pid1, &reg_path, &path_hash, terminate_flag, cleanup_flag);
        let wait_result = std::thread::scope(move |s| {
            let procs_handle = s.spawn(|| {
                std::thread::sleep(Duration::from_millis(100));
                stop_process(proc1);
                stop_process(proc2);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = procs_handle.join(); // should already have terminated
            wait_result
        });
        assert_eq!(wait_result, WaitResult::CleanUp);
    }

    #[test]
    fn terminates_on_shutdown_flag() {
        let proc = start_process();
        let pid = ActivationPid::from(proc.id() as i32);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid).unwrap();
        let mut watcher = MacOsWatcher::new(
            pid,
            &reg_path,
            &path_hash,
            terminate_flag.clone(),
            cleanup_flag.clone(),
        );
        let wait_result = std::thread::scope(move |s| {
            let flag_handle = s.spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                terminate_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert_eq!(wait_result, WaitResult::Terminate);
    }

    #[test]
    fn terminates_on_signal_handler_flag() {
        let proc = start_process();
        let pid = ActivationPid::from(proc.id() as i32);
        let (terminate_flag, cleanup_flag) = shutdown_flags();
        let path_hash = "abc";
        let reg_path = path_for_registry_with_entry(&path_hash);
        register_activation(&reg_path, &path_hash, pid).unwrap();
        let mut watcher = MacOsWatcher::new(
            pid,
            &reg_path,
            &path_hash,
            terminate_flag.clone(),
            cleanup_flag.clone(),
        );
        let wait_result = std::thread::scope(move |s| {
            let flag_handle = s.spawn(move || {
                std::thread::sleep(Duration::from_millis(100));
                cleanup_flag.store(true, std::sync::atomic::Ordering::SeqCst);
            });
            let watcher_handle = s.spawn(move || watcher.wait_for_termination().unwrap());
            let wait_result = watcher_handle.join().unwrap();
            let _ = flag_handle.join(); // should already have terminated
            wait_result
        });
        stop_process(proc);
        assert_eq!(wait_result, WaitResult::CleanUp);
    }
}
