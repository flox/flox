//# An attempt at defining a domain model for flox
pub mod environment;
pub mod environment_ref;
pub mod root;
pub use runix::{flake_ref, registry};
pub mod floxmeta;
pub mod floxmetav2;
pub mod lockfile;
pub mod manifest;
pub mod pkgdb;
pub mod search;
