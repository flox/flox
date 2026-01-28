use std::path::PathBuf;

use flox_core::activate::context::ActivateCtx;

use crate::env_diff::EnvDiff;
use crate::gen_rc::bash::BashStartupArgs;
use crate::gen_rc::fish::FishStartupArgs;
use crate::gen_rc::tcsh::TcshStartupArgs;
use crate::gen_rc::zsh::ZshStartupArgs;

pub mod bash;
pub mod fish;
pub mod tcsh;
pub mod zsh;

#[derive(Debug, Clone)]
pub enum StartupArgs {
    Bash(BashStartupArgs),
    Fish(FishStartupArgs),
    Tcsh(TcshStartupArgs),
    Zsh(ZshStartupArgs),
}

#[derive(Debug)]
pub struct StartupCtx {
    pub args: StartupArgs,
    pub rc_path: Option<PathBuf>,
    pub env_diff: EnvDiff,
    pub state_dir: PathBuf,
    pub act_ctx: ActivateCtx,
}
