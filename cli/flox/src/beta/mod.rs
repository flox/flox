//! Beta subsystems: everything gated behind the `features.beta` flag.
//!
//! The `Commands::Beta` arm in [`crate::commands`] checks `flox.features`
//! once before dispatching here, so handlers in this module don't need to
//! re-check it.

pub mod beta_enabled;
