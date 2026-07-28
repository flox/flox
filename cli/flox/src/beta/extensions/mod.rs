//! External-subcommand extension subsystem.
//!
//! `flox extension install|list|remove|search|upgrade` manages
//! `flox-<name>` executables under `flox.data_dir/extensions`, and
//! [`dispatch::try_dispatch_external`] makes `flox <name>` run one.
//!
//! See [`docs/`](./docs) for the user and author guides.
//!
//! The whole subsystem is gated behind `features.beta`, so it lives here
//! rather than in `flox-rust-sdk`: promoting or dropping the feature does
//! not churn a reviewed crate. It depends on `flox-rust-sdk` only for
//! [`Flox`] and the git provider.
//!
//! [`Flox`]: flox_rust_sdk::flox::Flox

pub mod commands;
pub mod dispatch;

pub(crate) mod archive;
pub(crate) mod extension;
pub(crate) mod github;
pub(crate) mod layout;
pub(crate) mod manager;
pub(crate) mod manifest;
pub mod reserved;
pub(crate) mod source;

pub use dispatch::try_dispatch_external;
// Re-exports are the surface the command handlers consume. As a crate this
// list was public API and could name types nothing called yet; as a module
// the compiler holds it to what is actually used, so keep it to that.
pub use extension::Extension;
pub use github::{SearchQuery, SearchSort, validate_owner};
pub use manager::{
    DryRunStatus,
    SearchRow,
    UpgradeStatus,
    install_github,
    install_local,
    list,
    remove,
    search,
    upgrade,
    upgrade_all,
    upgrade_all_dry_run,
    upgrade_dry_run,
};
pub use reserved::RESERVED_COMMAND_NAMES;
