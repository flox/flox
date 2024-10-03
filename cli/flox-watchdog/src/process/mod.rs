#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub(crate) use linux::*;
#[cfg(target_os = "macos")]
mod macos;
use std::time::Duration;

#[cfg(target_os = "macos")]
pub(crate) use macos::*;

/// How long to wait between watcher updates.
pub const WATCHER_SLEEP_INTERVAL: Duration = Duration::from_millis(100);

type Error = anyhow::Error;

#[derive(Debug, PartialEq, Eq)]
pub enum WaitResult {
    CleanUp,
    Terminate,
}

pub trait Watcher {
    /// Block while the watcher waits for a termination or cleanup event.
    fn wait_for_termination(&mut self) -> Result<WaitResult, Error>;
    /// Instructs the watcher to update the list of PIDs that it's watching
    /// by reading the environment registry (for now).
    fn update_watchlist(&mut self) -> Result<(), Error>;
    /// Returns true if the watcher determines that it's time to perform
    /// cleanup.
    fn should_clean_up(&self) -> Result<bool, Error>;
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::process::{Child, Command};
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    use flox_rust_sdk::models::env_registry::{EnvRegistry, RegistryEntry};
    use tempfile::NamedTempFile;

    // NOTE: these two functions are copied from flox-rust-sdk since you can't
    //       share anything behind #[cfg(test)] across crates

    /// Start a shortlived process that we can check the PID is running.
    pub fn start_process() -> Child {
        Command::new("sleep")
            .arg("2")
            .spawn()
            .expect("failed to start")
    }

    /// Stop a shortlived process that we can check the PID is not running. It's
    /// unlikely, but not impossible, that the kernel will have not re-used the
    /// PID by the time we check it.
    pub fn stop_process(mut child: Child) {
        child.kill().expect("failed to kill");
        child.wait().expect("failed to wait");
    }

    /// Makes two Arc<AtomicBool>s to mimic the shutdown flags used by
    /// the watchdog
    pub fn shutdown_flags() -> (Arc<AtomicBool>, Arc<AtomicBool>) {
        (
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Writes a registry to a temporary file, adding an entry for the provided
    /// path hash
    pub fn path_for_registry_with_entry(path_hash: impl AsRef<str>) -> NamedTempFile {
        let path = NamedTempFile::new().unwrap();
        let mut reg = EnvRegistry::default();
        let entry = RegistryEntry {
            path_hash: String::from(path_hash.as_ref()),
            path: PathBuf::from("foo"),
            envs: vec![],
            activations: HashSet::new(),
        };
        reg.entries.push(entry);
        let string = serde_json::to_string(&reg).unwrap();
        std::fs::write(&path, string).unwrap();
        path
    }
}
