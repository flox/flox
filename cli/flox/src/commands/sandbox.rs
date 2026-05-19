/// Sandbox introspection and management commands for the Flox Agent prototype.
///
/// The actual sandbox enforcement (sandbox-exec on macOS, bubblewrap on Linux)
/// is applied by the launcher process outside the sandboxed environment.  These
/// commands provide user-visible introspection for the running sandbox profile.
use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

use crate::utils::message;

#[derive(Bpaf, Clone, Debug)]
pub enum SandboxCommands {
    /// Show the active sandbox profile and its parameters
    #[bpaf(command)]
    Status,
}

impl SandboxCommands {
    pub fn handle(self, _flox: Flox) -> Result<()> {
        match self {
            SandboxCommands::Status => handle_status(),
        }
    }
}

fn handle_status() -> Result<()> {
    // Prototype: read the FLOX_SANDBOX_PROFILE env var if the launcher set it,
    // otherwise report that no sandbox is active.
    let profile = std::env::var("FLOX_SANDBOX_PROFILE").ok();
    let backend = std::env::var("FLOX_SANDBOX_BACKEND").ok();

    match (profile, backend) {
        (Some(profile), Some(backend)) => {
            message::plain(format!(
                "Sandbox active\n  Backend:  {backend}\n  Profile:  {profile}"
            ));
            // Report allowed paths from environment variables set by the launcher.
            for (key, label) in &[
                ("FLOX_SANDBOX_ALLOW_READ", "Read-only paths"),
                ("FLOX_SANDBOX_ALLOW_WRITE", "Read-write paths"),
                ("FLOX_SANDBOX_ALLOW_NET", "Network hosts"),
            ] {
                if let Ok(val) = std::env::var(key) {
                    if !val.is_empty() {
                        println!("  {label}:");
                        for item in val.split(':') {
                            println!("    {item}");
                        }
                    }
                }
            }
        },
        _ => {
            message::plain(
                "No sandbox active.\n  Start a sandboxed environment with 'flox activate --sandbox'.\n  (Flox Agent prototype — sandbox-exec on macOS, bubblewrap on Linux)",
            );
        },
    }

    Ok(())
}
