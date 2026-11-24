use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

use flox_core::util::nix_command;

static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("NIX_BIN")
        .unwrap_or_else(|_| env!("NIX_BIN").to_string())
        .into()
});

/// Returns a `Command` for `nix` with a default set of features enabled.
pub(crate) fn nix_base_command() -> Command {
    nix_command(&NIX_BIN)
}
