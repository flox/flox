pub mod providers;
pub mod utils;

pub mod models;

pub mod environment;

pub mod prelude {
    pub use crate::nix::installable::{FlakeAttribute, Installable};
}

pub mod flox;
pub use runix as nix;

pub mod data;
