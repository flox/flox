use std::path::PathBuf;

use flox_core::activate::context::ActivateCtx;

use crate::env_diff::EnvDiff;
use crate::gen_rc::bash::BashStartupArgs;

pub mod bash;
pub mod fish;
pub mod tcsh;
pub mod zsh;

#[derive(Debug, Clone)]
pub enum StartupArgs {
    Bash(BashStartupArgs),
}

#[derive(Debug)]
pub struct StartupCtx {
    pub args: StartupArgs,
    pub rc_path: Option<PathBuf>,
    pub env_diff: EnvDiff,
    pub state_dir: PathBuf,
    pub act_ctx: ActivateCtx,
}
