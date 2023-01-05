use std::path::PathBuf;

use anyhow::Result;
use bpaf::{construct, Bpaf, Parser, ShellComp};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::prelude::flox_package::FloxPackage;

use crate::config::Feature;
use crate::{flox_forward, should_flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    /// Path to environment.
    ///
    ///
    /// TODO: this will be changed to an environment name or an
    /// installable at some point, once we settle on how users specify environments
    #[bpaf(short, long, argument("ENV"))]
    pub environment: Option<PathBuf>,
}

impl EnvironmentCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            _ if should_flox_forward(Feature::Env)? => flox_forward(&flox).await?,

            EnvironmentCommands::Install {
                packages,
                environment: EnvironmentArgs { environment },
            } => {
                subcommand_metric!("install");

                flox.environment(environment.clone().unwrap())?
                    .install::<NixCommandLine>(packages)
                    .await?
            },

            _ => todo!(),
        }

        Ok(())
    }
}

fn activate_run_args() -> impl Parser<Option<(String, Vec<String>)>> {
    let command = bpaf::positional("COMMAND").strict();
    let args = bpaf::any("ARGUMENTS").many();

    bpaf::construct!(command, args).optional()
}

#[derive(Clone)]
pub enum ImportFile {
    Stdin,
    Path(PathBuf),
}

impl ImportFile {
    fn parse() -> impl Parser<ImportFile> {
        let stdin = bpaf::any::<char>("STDIN (-)")
            .help("Use `-` to read from STDIN")
            .complete(|_| vec![("-", Some("Read from STDIN"))])
            .guard(|t| *t == '-', "Use `-` to read from STDIN")
            .map(|_| ImportFile::Stdin);
        let path = bpaf::positional("PATH")
            .help("Path to export file")
            .complete_shell(ShellComp::File { mask: None })
            .map(ImportFile::Path);
        construct!([stdin, path])
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
    Activate {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(external(activate_run_args))]
        arguments: Option<(String, Vec<String>)>,
    },

    /// remove all data pertaining to an environment`
    #[bpaf(command)]
    Destroy {
        #[bpaf(short, long)]
        force: bool,

        #[bpaf(short, long)]
        origin: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// edit declarative environment configuration
    #[bpaf(command)]
    Edit {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// export declarative environment manifest to STDOUT
    #[bpaf(command)]
    Export {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// list environment generations with contents
    #[bpaf(command)]
    Generations {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// access to the git CLI for floxmeta repository
    #[bpaf(command)]
    Git {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(any("Git Arguments"))]
        git_arguments: Vec<String>,
    },

    /// show all versions of an environment
    #[bpaf(command)]
    History {
        #[bpaf(long, short)]
        oneline: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// import declarative environment manifest from STDIN as new generation
    #[bpaf(command)]
    Import {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(external(ImportFile::parse), fallback(ImportFile::Stdin))]
        file: ImportFile,
    },

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
    List {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(external(list_output), optional)]
        json: Option<ListOutput>,

        /// The generation to list, if not speciefied defaults to the current one
        #[bpaf(positional("GENERATION"))]
        generation: Option<u32>,
    },

    /// pull environment metadata from remote registry
    #[bpaf(command)]
    Push {
        #[bpaf(long, short)]
        force: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// send environment metadata to remote registry
    #[bpaf(command)]
    Pull {
        #[bpaf(long, short)]
        force: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },

    /// remove packages from an environment
    #[bpaf(command, long("rm"))]
    Remove {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<FloxPackage>,
    },

    /// rollback to the previous generation of an environment
    #[bpaf(command)]
    Rollback {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        /// Generation to roll back to.
        ///
        /// If omitted, defaults to the previous generation.
        #[bpaf(argument("GENERATION"))]
        to: Option<u32>,
    },

    /// switch to a specific generation of an environment
    #[bpaf(command("switch-generation"))]
    SwitchGeneration {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(positional("GENERATION"))]
        generation: u32,
    },

    /// upgrade packages using their most recent flake
    #[bpaf(command)]
    Upgrade {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,

        #[bpaf(positional("PACKAGES"))]
        packages: Vec<FloxPackage>,
    },

    /// delete non-current versions of an environment
    #[bpaf(command("wipe-history"))]
    WipeHistory {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment: EnvironmentArgs,
    },
}

#[derive(Bpaf, Clone)]
pub enum ListOutput {
    /// Include store paths of packages in the environment
    #[bpaf(long("out-path"))]
    OutPath,
    /// Print as machine readable json
    #[bpaf(long)]
    Json,
}
