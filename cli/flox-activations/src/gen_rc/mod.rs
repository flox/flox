use std::path::PathBuf;

use flox_core::activate::context::{ActivateCoreCtx, ActivateProjectCtx};

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

/// Context for shell startup, shared between normal and container activations.
#[derive(Debug)]
pub struct StartupCtx {
    pub args: StartupArgs,
    pub rc_path: Option<PathBuf>,
    pub env_diff: EnvDiff,
    pub state_dir: PathBuf,
    /// Core activation context (always present)
    pub core: ActivateCoreCtx,
    /// Project context (None for containers)
    pub project: Option<ActivateProjectCtx>,
}
