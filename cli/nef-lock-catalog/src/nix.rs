use std::path::PathBuf;
use std::process::Command;
use std::sync::LazyLock;

// Note: duplicated from `flox-rust-sdk/src/providers/nix.rs`,
// as this crate was originally built as a standalone crate
// without dependence on `flox-rust-sdk` or as part of that crate,
// to avoid further complicating it.

static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("NIX_BIN")
        .unwrap_or_else(|_| env!("NIX_BIN").to_string())
        .into()
});

/// Returns a `Command` for `nix` with a default set of features enabled.
pub(crate) fn nix_base_command() -> Command {
    let mut command = Command::new(&*NIX_BIN);
    command.args([
        "--option",
        "extra-experimental-features",
        "nix-command flakes",
    ]);
    command
}
