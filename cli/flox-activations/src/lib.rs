pub mod attach;
pub mod attach_diff;
pub mod cli;
pub mod deactivate;
pub mod env_diff;
pub mod env_trace;
pub mod gen_rc;
pub mod hook;
pub mod logger;
pub mod message;
mod process_compose;
mod start;
mod vars_from_env;

pub type Error = anyhow::Error;
