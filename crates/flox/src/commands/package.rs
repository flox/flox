use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

use crate::flox_forward;

#[derive(Bpaf)]
pub struct PackageArgs {
    stability: Option<Stability>,

    #[bpaf(external(package_commands))]
    command: PackageCommands,

    #[bpaf(positional("INSTALLABLE"))]
    installable: String,
}

impl PackageArgs {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match &self.command {
            PackageCommands::Build { .. }
            | PackageCommands::Develop { .. }
            | PackageCommands::Publish { .. }
            | PackageCommands::Run { .. }
            | PackageCommands::Shell { .. } => flox_forward().await?,
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone)]
#[bpaf(adjacent)]
pub enum PackageCommands {
    /// build package from current project
    #[bpaf(command)]
    Build {},

    /// launch development shell for current project
    #[bpaf(command)]
    Develop {},
    /// build and publish project to flox channel
    #[bpaf(command)]
    Publish {
        /// The --upstream-url determines the upstream repository containing
        #[bpaf(argument("REPO"))]
        channel_repo: String,
    },
    /// run app from current project
    #[bpaf(command)]
    Run {},
    /// run a shell in which the current project is available
    #[bpaf(command)]
    Shell {},
}
