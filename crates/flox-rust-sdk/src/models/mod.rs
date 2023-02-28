//# An attempt at defining a domain model for flox

pub mod channels;
pub mod environment_ref;
pub mod flox_installable;
pub mod flox_package;
pub mod root;
pub use runix::{flake_ref, registry};
pub mod stability;
