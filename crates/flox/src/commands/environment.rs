use std::path::PathBuf;

use anyhow::{Context, Result};
use bpaf::{construct, Bpaf, Parser, ShellComp};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::CommonEnvironment;
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::models::floxmeta::Floxmeta;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::prelude::flox_package::FloxPackage;
use flox_rust_sdk::providers::git::{GitCommandProvider, GitProvider};
use flox_types::constants::{DEFAULT_CHANNEL, LATEST_VERSION};
use itertools::Itertools;
use serde_json::json;

use crate::config::features::Feature;
use crate::utils::resolve_environment_ref;
use crate::{flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
}

pub type EnvironmentRef = PathBuf;

impl EnvironmentCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            EnvironmentCommands::List {
                environment_args: _,
                environment,
                json: _,
                generation: _,
            } if !Feature::Env.is_forwarded()? => {
                let environment_name = environment.as_ref().map(|e| e.to_str().unwrap());
                let environment_ref: environment_ref::EnvironmentRef =
                    resolve_environment_ref::<GitCommandProvider>(&flox, "list", environment_name)
                        .await?;

                let environment = environment_ref
                    .to_env::<GitCommandProvider, NixCommandLine>(&flox)
                    .await
                    .context("Environment not found")?;

                match environment {
                    CommonEnvironment::Named(env) => {
                        let generation = env.generation(Default::default()).await?;
                        println!("{}", serde_json::to_string_pretty(&generation).unwrap())
                    },
                    CommonEnvironment::Project(env) => {
                        let catalog = env.catalog(&flox).await?;
                        let installed_store_paths = env.installed_store_paths(&flox).await?;

                        println!("Packages in {env}:");
                        for (publish_element, _) in catalog.entries.iter() {
                            if publish_element.version != LATEST_VERSION {
                                println!(
                                    "{} {}",
                                    publish_element.to_flox_tuple(),
                                    publish_element.version
                                )
                            } else {
                                println!("{}", publish_element.to_flox_tuple())
                            }
                        }
                        for store_path in installed_store_paths.iter() {
                            println!("{}", store_path.to_string_lossy())
                        }
                    },
                }
            },

            EnvironmentCommands::Envs if !Feature::Env.is_forwarded()? => {
                let floxmetas = Floxmeta::<GitCommandProvider, _>::list_floxmetas(&flox).await?;

                let mut values = Vec::new();

                for meta in floxmetas {
                    let envs = meta.environments().await?;
                    let mut dir = meta.git().workdir();
                    let dir = dir.get_or_insert_with(|| meta.git().path());

                    values.push(json!({
                        "type": "floxmeta",
                        "path": dir,
                        "envs": envs,
                    }));
                }

                println!("{}", serde_json::to_string_pretty(&values)?);
            },

            EnvironmentCommands::Install {
                packages,
                environment_args: EnvironmentArgs { .. },
                environment,
            } if !Feature::Env.is_forwarded()? => {
                subcommand_metric!("install");

                let packages: Vec<_> = packages
                    .iter()
                    .map(|package| FloxPackage::parse(package, &flox.channels, DEFAULT_CHANNEL))
                    .try_collect()?;

                flox.environment(environment.clone().unwrap())?
                    .install::<NixCommandLine>(&packages)
                    .await?
            },

            _ => flox_forward(&flox).await?,
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
pub enum PullFloxmainOrEnv {
    /// pull the `floxmain` branch to sync configuration
    #[bpaf(long, short)]
    Main,
    Env {
        #[bpaf(long("environment"), short('e'), argument("ENV"))]
        env: Option<EnvironmentRef>,
        /// do not actually render or create links to environments in the store.
        /// (Flox internal use only.)
        #[bpaf(long("no-render"))]
        no_render: bool,
    },
}

#[derive(Bpaf, Clone)]
pub enum PushFloxmainOrEnv {
    /// push the `floxmain` branch to sync configuration
    #[bpaf(long, short)]
    Main,
    Env {
        #[bpaf(long("environment"), short('e'), argument("ENV"))]
        env: Option<EnvironmentRef>,
    },
}

#[derive(Bpaf, Clone)]
pub enum EnvironmentCommands {
    /// list all available environments
    /// Aliases:
    ///   environments, envs
    #[bpaf(command, long("environments"))]
    Envs,

    /// activate environment:
    ///
    /// * in current shell: . <(flox activate)
    /// * in subshell: flox activate
    /// * for command: flox activate -- <command> <args>
    #[bpaf(command)]
    Activate {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Vec<EnvironmentRef>,

        #[bpaf(external(activate_run_args))]
        arguments: Option<(String, Vec<String>)>,
    },

    /// create an environment
    #[bpaf(command)]
    Create {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        // Indicates that `templateFloxEnvNoProfiles` should be used as template
        // for environment instead of `templateFloxEnv`.
        #[bpaf(long("no-profiles"), short('P'))]
        no_profiles: bool,
    },

    /// remove all data pertaining to an environment
    #[bpaf(command)]
    Destroy {
        #[bpaf(short, long)]
        force: bool,

        #[bpaf(short, long)]
        origin: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
    },

    /// edit declarative environment configuration
    #[bpaf(command)]
    Edit {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
    },

    /// export declarative environment manifest to STDOUT
    #[bpaf(command)]
    Export {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
    },

    /// list environment generations with contents
    #[bpaf(command)]
    Generations {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long)]
        json: bool,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
    },

    /// access to the git CLI for floxmeta repository
    #[bpaf(command)]
    Git {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(any("Git Arguments"))]
        git_arguments: Vec<String>,
    },

    /// show all versions of an environment
    #[bpaf(command)]
    History {
        #[bpaf(long, short)]
        oneline: bool,

        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
    },

    /// import declarative environment manifest from STDIN as new generation
    #[bpaf(command)]
    Import {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(external(ImportFile::parse), fallback(ImportFile::Stdin))]
        file: ImportFile,
    },

    /// install a package into an environment
    #[bpaf(command)]
    Install {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<String>,
    },

    /// list packages installed in an environment
    #[bpaf(command)]
    List {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(external(list_output), optional)]
        json: Option<ListOutput>,

        /// The generation to list, if not specified defaults to the current one
        #[bpaf(positional("GENERATION"))]
        generation: Option<u32>,
    },

    /// send environment metadata to remote registry
    #[bpaf(command)]
    Push {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(external(push_floxmain_or_env), optional)]
        target: Option<PushFloxmainOrEnv>,

        /// forceably overwrite the remote copy of the environment
        #[bpaf(long, short)]
        force: bool,
    },

    /// pull environment metadata from remote registry
    #[bpaf(command)]
    Pull {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(external(pull_floxmain_or_env), optional)]
        target: Option<PullFloxmainOrEnv>,

        /// forceably overwrite the local copy of the environment
        #[bpaf(long, short)]
        force: bool,
    },

    /// remove packages from an environment
    #[bpaf(command, long("rm"))]
    Remove {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(positional("PACKAGES"), some("At least one package"))]
        packages: Vec<String>,
    },

    /// rollback to the previous generation of an environment
    #[bpaf(command)]
    Rollback {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

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
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(positional("GENERATION"))]
        generation: u32,
    },

    /// upgrade packages using their most recent flake
    #[bpaf(command)]
    Upgrade {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,

        #[bpaf(positional("PACKAGES"))]
        packages: Vec<String>,
    },

    /// delete non-current versions of an environment
    #[bpaf(command("wipe-history"))]
    WipeHistory {
        #[bpaf(external(environment_args), group_help("Environment Options"))]
        environment_args: EnvironmentArgs,

        #[bpaf(long, short, argument("ENV"))]
        environment: Option<EnvironmentRef>,
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
