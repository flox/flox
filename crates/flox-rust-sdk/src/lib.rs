extern crate pretty_env_logger;
#[macro_use]
extern crate log;
#[macro_use]
extern crate lazy_static;

pub mod providers;
pub mod utils;

mod models;

pub mod environment;

pub mod prelude {
    pub use crate::models::channels::{Channel, ChannelRegistry};
    pub use crate::models::{catalog::Stability, flox_package};
    pub use crate::nix::installable::Installable;
}

pub mod actions;
pub mod flox;
pub use runix as nix;
