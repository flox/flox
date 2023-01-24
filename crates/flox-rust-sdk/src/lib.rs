pub mod providers;
pub mod utils;

pub mod models;

pub mod environment;

pub mod prelude {
    pub use crate::models::channels::{Channel, ChannelRegistry};
    pub use crate::models::flox_package;
    pub use crate::models::stability::Stability;
    pub use crate::nix::installable::Installable;
}

pub mod actions;
pub mod flox;
pub use runix as nix;
