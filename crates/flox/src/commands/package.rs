use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

use crate::{config::Config, flox_forward};

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
            _ if !Config::preview_enabled()? => flox_forward().await?,
            PackageCommands::Build {} => {
                flox.package(
                    self.installable.clone().into(),
                    self.stability.clone().unwrap_or_default(),
                )
                .build::<NixCommandLine>()
                .await?
            }

            PackageCommands::Develop {} => {
                flox.package(
                    self.installable.clone().into(),
                    self.stability.clone().unwrap_or_default(),
                )
                .develop::<NixCommandLine>()
                .await?
            }
            _ => todo!(),
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
