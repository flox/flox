use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

use crate::flox_forward;

#[derive(Bpaf)]
pub struct GeneralArgs {
    #[bpaf(external(general_commands))]
    command: GeneralCommands,
}

impl GeneralArgs {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match &self.command {
            GeneralCommands::Init { .. }
            | GeneralCommands::Gh { .. }
            | GeneralCommands::Nix { .. }
            | GeneralCommands::Config { .. }
            | GeneralCommands::Envs => flox_forward().await?,
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(adjacent)]
pub enum GeneralCommands {
    /// initialize flox expressions for current project
    #[bpaf(command)]
    Init {},

    ///access to the gh CLI
    #[bpaf(command)]
    Gh(Vec<String>),

    #[bpaf(command, hide)]
    Nix(Vec<String>),

    /// configure user parameters
    #[bpaf(command)]
    Config,

    /// list all available environments
    #[bpaf(command, long("environments"))]
    Envs,
}

pub type ChannelRef = String;
pub type Url = String;
