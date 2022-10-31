extern crate pretty_env_logger;
#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;

pub mod config;
pub mod providers;
pub mod utils;

mod models;

pub mod environment;

pub mod prelude {
    pub use super::models::*;
    pub use crate::flox::Flox;
}

pub mod actions;
pub mod flox;
pub mod nix;
