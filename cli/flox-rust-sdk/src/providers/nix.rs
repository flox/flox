use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("NIX_BIN")
        .unwrap_or_else(|_| env!("NIX_BIN").to_string())
        .into()
});

/// Returns a `Command` for `nix` with a default set of features enabled.
pub fn nix_base_command() -> Command {
    let mut command = Command::new(&*NIX_BIN);
    command.args([
        "--option",
        "extra-experimental-features",
        "nix-command flakes",
    ]);
    command
}

pub mod test_helpers {
    use std::path::PathBuf;

    use super::*;

    /// Returns a Nix store path that's known to exist.
    pub fn known_store_path() -> PathBuf {
        NIX_BIN.to_path_buf()
    }
}
