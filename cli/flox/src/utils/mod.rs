use std::env;
use std::io::Stderr;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flox_core::util::default_nix_env_vars;

pub mod active_environments;
pub mod colors;
pub mod dialog;
pub mod didyoumean;
pub mod errors;
pub mod init;
pub mod message;
pub mod metrics;
pub mod openers;
pub mod search;
pub mod tracing;
pub mod update_notifications;

pub static TERMINAL_STDERR: LazyLock<Mutex<Stderr>> =
    LazyLock::new(|| Mutex::new(std::io::stderr()));
/// Timeout used for network operations that run after the main flox command has
/// completed.
///
/// This is used for metrics submission and checking for updates.
pub const TRAILING_NETWORK_CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// Set the default nix environment variables for the current process
///
/// SAFETY: called once, prior to possible concurrent access to env
pub fn populate_default_nix_env_vars() {
    let env_map = default_nix_env_vars();
    for (key, value) in env_map {
        unsafe { env::set_var(key, value) }
    }
}
