pub mod activation_diff;
pub mod attach;
pub mod attach_diff;
pub mod cli;
pub mod gen_rc;
pub mod hook;
pub mod logger;
pub mod message;
mod process_compose;
mod start;
pub mod start_diff;
mod vars_from_env;

pub type Error = anyhow::Error;
