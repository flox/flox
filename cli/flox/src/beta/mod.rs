//! Beta subsystems: everything gated behind the `features.beta` flag.
//!
//! Code under this module is held to a lighter review bar than the rest of
//! the CLI. It was originally a separate `beta` crate to enforce that
//! boundary mechanically, but the isolation cost more than it bought: beta
//! code could not reach `utils::message`, `subcommand_metric!`, or
//! `active_environments` without duplicating them or hoisting them into a
//! shared crate. The boundary is now this module, by convention.
//!
//! The `Commands::Beta` arm in [`crate::commands`] checks `flox.features`
//! once before dispatching here, so handlers in this module must not
//! re-check it.

pub mod beta_enabled;
pub mod extensions;
