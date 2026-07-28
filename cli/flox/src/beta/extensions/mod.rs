//! External-subcommand extension subsystem.
//!
//! See [`research/gh_extension_flox.md`](../../../../../research/gh_extension_flox.md)
//! for the full PRD and design.
//!
//! This lives in the `beta` module rather than `flox-rust-sdk` because the
//! whole subsystem is gated behind `features.beta`: keeping it here leaves
//! the SDK untouched, so promoting or dropping the feature does not churn a
//! reviewed crate. It depends on `flox-rust-sdk` only for [`Flox`] and the
//! git provider.
//!
//! [`Flox`]: flox_rust_sdk::flox::Flox

pub mod dispatch;

pub(crate) mod archive;
pub(crate) mod extension;
pub(crate) mod github;
pub(crate) mod layout;
pub(crate) mod manager;
pub(crate) mod manifest;
pub mod reserved;
pub(crate) mod source;

// Re-exports are the surface the CLI handlers consume. As a crate this list
// was public API and could name types nothing called yet; as a module the
// compiler holds it to what is actually used, so keep it to that.
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
pub use manifest::{AuthorManifest, InstalledState, parse_author_manifest, parse_installed_state};
pub use reserved::RESERVED_COMMAND_NAMES;
