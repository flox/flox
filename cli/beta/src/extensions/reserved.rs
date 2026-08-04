//! Top-level subcommand names that conflict with the `flox <name>`
//! external-extension dispatch.
//!
//! `try_dispatch_external` only fires when bpaf fails to parse the first
//! positional as a known subcommand, so any subcommand name shadows an
//! extension of the same name. To prevent surprises like
//! `flox install` silently dispatching to a user's `flox-install`
//! extension if bpaf's parser ever changes, `install_github` rejects any
//! repo whose `<name>` segment is in this list.
//!
//! This list is verified by a drift test in `cli/tests/extension.bats`
//! ("reserved-name list covers every visible top-level command"), which
//! parses `flox --help` and asserts each command is refused at install
//! time. If a new visible top-level command is added, that test fails and
//! points here.
//!
//! Hidden commands are invisible to `--help` and so cannot be caught by
//! that test; they must be added here by hand. As of the rebase onto
//! `dd390e66b` those are `extension`, `help`, `beta-enabled`, and
//! `factory`.

pub const RESERVED_COMMAND_NAMES: &[&str] = &[
    // Manage
    "init",
    "envs",
    "delete",
    // Use
    "activate",
    "deactivate",
    "run",
    "services",
    // Discover
    "search",
    "show",
    // Modify
    "install",
    "i",
    "list",
    "l",
    "edit",
    "include",
    "upgrade",
    "uninstall",
    "generations",
    // Share
    "build",
    "publish",
    "push",
    "pull",
    "containerize",
    // Administration
    "auth",
    "config",
    "gc",
    // Hidden / internal (not in --help output, so not covered by the
    // bats drift test — keep this section current by hand)
    "extension",
    "help",
    "beta-enabled",
    "factory",
];
