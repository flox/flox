use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::{flox::Flox, nix::command_line::NixCommandLine, prelude::Stability};

use crate::{config::Config, flox_forward};

#[derive(Bpaf, Clone)]

pub struct PackageArgs {
    stability: Option<Stability>,

    #[bpaf(short('A'), argument("INSTALLABLE"))]
    installable: Option<String>,
}

impl PackageCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if !Config::preview_enabled()? => flox_forward().await?,
            PackageCommands::Build {
                package:
                    PackageArgs {
                        stability,
                        installable,
                    },
            } => {
                flox.package(
                    installable.clone().unwrap().into(),
                    stability.clone().unwrap_or_default(),
                )
                .build::<NixCommandLine>()
                .await?
            }

            PackageCommands::Develop {
                package:
                    PackageArgs {
                        stability,
                        installable,
                    },
            } => {
                flox.package(
                    installable.clone().unwrap().into(),
                    stability.clone().unwrap_or_default(),
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
pub enum PackageCommands {
    /// initialize flox expressions for current project
    #[bpaf(command)]
    Init {},

    /// build package from current project
    #[bpaf(command)]
    Build {
        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },

    /// launch development shell for current project
    #[bpaf(command)]
    Develop {
        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// build and publish project to flox channel
    #[bpaf(command)]
    Publish {
        /// The --upstream-url determines the upstream repository containing
        #[bpaf(argument("REPO"))]
        channel_repo: String,

        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// run app from current project
    #[bpaf(command)]
    Run {
        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
    /// run a shell in which the current project is available
    #[bpaf(command)]
    Shell {
        #[bpaf(external(package_args), group_help("Development Options"))]
        package: PackageArgs,
    },
}
