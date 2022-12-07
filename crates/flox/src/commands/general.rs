use std::future::Future;

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::{
    flox::Flox,
    nix::{
        arguments::NixArgs,
        Run,
    },
    prelude::Stability,
};
use log::debug;
use tempfile::{tempfile, TempDir};

use crate::{commands::channel, config::Config, flox_forward, utils::init_channels};

#[derive(Bpaf, Clone)]
pub struct GeneralArgs {}

impl GeneralCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if !Config::preview_enabled()? => flox_forward().await?,
            _ => todo!(),
        }
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
pub enum GeneralCommands {
    /// initialize flox expressions for current project
    #[bpaf(command)]
    Init {},

    ///access to the gh CLI
    #[bpaf(command)]
    Gh(Vec<String>),

    #[bpaf(command)]
    Nix(#[bpaf(positional, complete_shell(complete_nix_shell()))] Vec<String>),

    /// configure user parameters
    #[bpaf(command)]
    Config,

    /// list all available environments
    #[bpaf(command, long("environments"))]
    Envs,
}

fn complete_nix_shell() -> bpaf::ShellComp {
    // Box::leak will effectively turn the String
    // (that is produced by `replace`) insto a `&'static str`,
    // at the cost of giving up memeory management over that string.
    //
    // Note:
    // We could use a `OnceCell` to ensure this leak happens only once.
    // However, this should not be necessary after all,
    // since the completion runs in its own process.
    // Any memory it leaks will be cleared by the system allocator.
    bpaf::ShellComp::Raw {
        zsh: Box::leak(format!("source {}", env!("NIX_ZSH_COMPLETION_SCRIPT")).into_boxed_str()),
        bash: Box::leak(
            format!(
                "source {}; _nix_bash_completion",
                env!("NIX_BASH_COMPLETION_SCRIPT")
            )
            .into_boxed_str(),
        ),
        fish: "",
        elvish: "",
    }
}

pub type ChannelRef = String;
pub type Url = String;
