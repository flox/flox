extern crate pretty_env_logger;
#[macro_use]
extern crate log;

pub mod providers;
pub mod utils;

mod models;

pub mod environment;

pub mod prelude {
    pub use crate::models::catalog::Stability;
    pub use crate::models::channels::{Channel, ChannelRegistry};
    pub use crate::nix::installable::Installable;
}

pub mod actions;
pub mod flox;
pub use runix as nix;
