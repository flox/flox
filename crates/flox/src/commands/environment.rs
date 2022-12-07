use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::prelude::flox_package::FloxPackage;
use std::path::PathBuf;

use crate::config::Config;
use crate::flox_forward;

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    /// path to environment. TODO: this will be changed to an environment name or an
    /// installable at some point, once we settle on how users specify environments
    #[bpaf(short('e'))]
    pub environment: PathBuf,
}

impl EnvironmentCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if !Config::preview_enabled()? => flox_forward().await?,
            EnvironmentCommands::Install {
                packages,
                environment: EnvironmentArgs { environment },
            } => {
                flox.environment(environment.clone())?
                    .install::<NixCommandLine>(packages)
                    .await?
            }

            _ => todo!(),
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone)]
pub enum EnvironmentCommands {
    /// activate environment:
    ///
    /// * in current shell: . <(flox activate)
    /// * in subshell: flox activate
    /// * for command: flox activate -- <command> <args>
    #[bpaf(command)]
    Activate(Vec<String>),

    /// display declarative environment manifest
    #[bpaf(command)]
    Cat,

    /// remove all data pertaining to an environment
    #[bpaf(command)]
    Destroy,

    /// edit declarative environment configuration
    #[bpaf(command)]
    Edit,

    /// list environment generations with contents
    #[bpaf(command)]
    Generations,

    /// access to the git CLI for floxmeta repository
    #[bpaf(command)]
    Git(Vec<String>),

    /// show all versions of an environment
    #[bpaf(command)]
    History,

    /// install a package into an environment
    #[bpaf(command)]
    Install {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<FloxPackage>,
    },

    /// list packages installed in an environment
    #[bpaf(command)]
    List,

    /// pull environment metadata from remote registry
    #[bpaf(command)]
    Push { force: bool },

    /// send environment metadata to remote registry
    #[bpaf(command)]
    Pull { force: bool },

    /// remove packages from an environment
    #[bpaf(command)]
    Remove {
        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<FloxPackage>,
    },

    /// rollback to the previous generation of an environment
    #[bpaf(command)]
    Rollback,

    /// switch to a specific generation of an environment
    #[bpaf(command)]
    SwitchGeneration {
        #[bpaf(positional("GENERATION"))]
        generation: u32,
    },

    /// upgrade packages using their most recent flake
    #[bpaf(command)]
    Upgrade {
        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<FloxPackage>,
    },
}
