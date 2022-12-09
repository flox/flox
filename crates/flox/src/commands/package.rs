use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::{
    flox::{Flox, FloxInstallable, ParseFloxInstallableError},
    nix::command_line::NixCommandLine,
    prelude::Stability,
};

use crate::{config::Config, flox_forward, utils::resolve_installable};

fn bpaf_parse_flox_installable(
    installable: String,
) -> Result<FloxInstallable, ParseFloxInstallableError> {
    Ok(installable.parse()?)
}

#[derive(Bpaf, Clone)]

pub struct PackageArgs {
    stability: Option<Stability>,

    #[bpaf(
        short('A'),
        argument::<String>("INSTALLABLE"),
        parse(bpaf_parse_flox_installable),
        optional
    )]
    arg_installable: Option<FloxInstallable>,
    #[bpaf(positional::<String>("INSTALLABLE"), parse(bpaf_parse_flox_installable), optional)]
    pos_installable: Option<FloxInstallable>,

    #[bpaf(external(nix_args))]
    nix_arguments: Vec<String>,
}

impl PackageArgs {
    fn installable(&self) -> FloxInstallable {
        self.arg_installable
            .as_ref()
            .or(self.pos_installable.as_ref())
            .unwrap_or(&FloxInstallable {
                source: None,
                attr_path: vec![],
            })
            .clone()
    }
}

fn nix_args() -> impl Parser<Vec<String>> {
    extra_args("NIX ARGUMENTS")
}

fn extra_args(var: &'static str) -> impl Parser<Vec<String>> {
    bpaf::any(var)
        .guard(|m| m != "--help", "Not A Nix Arg")
        .many()
}

impl PackageCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if !Config::preview_enabled()? => flox_forward().await?,
            PackageCommands::Build {
                package:
                    package @ PackageArgs {
                        stability,
                        nix_arguments,
                        ..
                    },
            } => {
                let installable = resolve_installable(
                    &flox,
                    package.installable(),
                    &["."],
                    &[("packages", true)],
                )
                .await?;

                flox.package(
                    installable.into(),
                    stability.clone().unwrap_or_default(),
                    nix_arguments.clone(),
                )
                .build::<NixCommandLine>()
                .await?
            }

            PackageCommands::Develop {
                package:
                    package @ PackageArgs {
                        stability,
                        nix_arguments,
                        ..
                    },
            } => {
                let installable = resolve_installable(
                    &flox,
                    package.installable(),
                    &["."],
                    &[("packages", true), ("devShells", true)],
                )
                .await?;

                flox.package(
                    installable.into(),
                    stability.clone().unwrap_or_default(),
                    nix_arguments.clone(),
                )
                .develop::<NixCommandLine>()
                .await?
            }
            PackageCommands::Run {
                package:
                    package @ PackageArgs {
                        stability,
                        nix_arguments,
                        ..
                    },
            } => {
                let installable = resolve_installable(
                    &flox,
                    package.installable(),
                    &["."],
                    &[("packages", true), ("apps", true)],
                )
                .await?;

                flox.package(
                    installable.into(),
                    stability.clone().unwrap_or_default(),
                    nix_arguments.clone(),
                )
                .run::<NixCommandLine>()
                .await?
            }
            PackageCommands::Shell {
                package:
                    package @ PackageArgs {
                        stability,
                        nix_arguments,
                        ..
                    },
            } => {
                let installable = resolve_installable(
                    &flox,
                    package.installable(),
                    &["."],
                    &[("packages", true)],
                )
                .await?;

                flox.package(
                    installable.into(),
                    stability.clone().unwrap_or_default(),
                    nix_arguments.clone(),
                )
                .shell::<NixCommandLine>()
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
