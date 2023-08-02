use std::path::PathBuf;

use anyhow::{Context, Result};
use bpaf::{construct, Bpaf, Parser, ShellComp};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{DotFloxDir, Environment, EnvironmentError2, Read};
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::command::{Shell, StoreGc};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::flox_package::FloxPackage;
use flox_types::constants::{DEFAULT_CHANNEL, LATEST_VERSION};
use itertools::Itertools;
use log::info;

use crate::config::features::Feature;
use crate::utils::resolve_environment_ref;
use crate::{flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
}

pub type EnvironmentRef = String;

impl EnvironmentCommands {
    pub async fn handle(&self, flox: Flox) -> Result<()> {
        match self {
            EnvironmentCommands::List { .. } => subcommand_metric!("list"),
            EnvironmentCommands::Envs => subcommand_metric!("envs"),
            EnvironmentCommands::Activate { .. } => subcommand_metric!("activate"),
            EnvironmentCommands::Create { .. } => subcommand_metric!("create"),
            EnvironmentCommands::Destroy { .. } => subcommand_metric!("destroy"),
            EnvironmentCommands::Edit { .. } => subcommand_metric!("edit"),
            EnvironmentCommands::Export { .. } => subcommand_metric!("export"),
            EnvironmentCommands::Generations { .. } => subcommand_metric!("generations"),
            EnvironmentCommands::Git { .. } => subcommand_metric!("git"),
            EnvironmentCommands::History { .. } => subcommand_metric!("history"),
            EnvironmentCommands::Import { .. } => subcommand_metric!("import"),
            EnvironmentCommands::Install { .. } => subcommand_metric!("install"),
            EnvironmentCommands::Push { .. } => subcommand_metric!("push"),
            EnvironmentCommands::Pull { .. } => subcommand_metric!("pull"),
            EnvironmentCommands::Remove { .. } => subcommand_metric!("remove"),
            EnvironmentCommands::Rollback { .. } => subcommand_metric!("rollback"),
            EnvironmentCommands::SwitchGeneration { .. } => subcommand_metric!("switch"),
            EnvironmentCommands::Upgrade { .. } => subcommand_metric!("upgrade"),
            EnvironmentCommands::WipeHistory { .. } => subcommand_metric!("wipe-history"),
        }

        match self {
            EnvironmentCommands::Activate {
                environment_args: _,
                environment,
                arguments: _,
            } if !Feature::Env.is_forwarded()? => {
                let environment = environment.first().map(|e| e.as_ref());
                let environment = resolve_environment(&flox, environment, "install").await?;

                let command = Shell {
                    eval: EvaluationArgs {
                        impure: true.into(),
                    },
                    installables: [environment.flake_attribute(&flox.system).into()].into(),
                    ..Default::default()
                };

                let nix = flox.nix(Default::default());
                command.run(&nix, &Default::default()).await?
            },

            EnvironmentCommands::Create {
                environment_args: _,
                environment,
            } => {
                let current_dir = std::env::current_dir().unwrap();

                let mut dot_flox_dir = match DotFloxDir::open(&current_dir) {
                    Ok(d) => d,
                    Err(EnvironmentError2::NoDotFloxFound) => DotFloxDir::new(&current_dir)?,
                    Err(e) => Err(e)?,
                };

                let env = dot_flox_dir
                    .create_env(environment.as_deref().unwrap_or("default"))
                    .await?;

                println!(
                    "Created environment {name} in {path:?}",
                    name = environment_ref::EnvironmentRef::from(env),
                    path = current_dir
                );
            },

            EnvironmentCommands::List {
                environment_args: _,
                environment,
                json: _,
                generation: _,
            } if !Feature::Env.is_forwarded()? => {
                let env = resolve_environment(&flox, environment.as_deref(), "install").await?;

                let catalog = env
                    .catalog(&flox.nix(Default::default()), &flox.system)
                    .await
                    .context("Could not get catalog")?;
                // let installed_store_paths = env.installed_store_paths(&flox).await?;

                println!(
                    "Packages in {}:",
                    environment_ref::EnvironmentRef::from(env)
                );
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
                // for store_path in installed_store_paths.iter() {
                //     println!("{}", store_path.to_string_lossy())
                // }
            },

            EnvironmentCommands::Envs if !Feature::Env.is_forwarded()? => {
                let dot_flox_dir = DotFloxDir::discover(std::env::current_dir().unwrap())?;
                let envs = dot_flox_dir
                    .environments()?
                    .into_iter()
                    .map(environment_ref::EnvironmentRef::from);

                println!("Envs in {:?}", dot_flox_dir.path());
                for env in envs {
                    println!("- {env}");
                }
            },

            EnvironmentCommands::Install {
                packages,
                environment_args: EnvironmentArgs { .. },
                environment,
            } if !Feature::Env.is_forwarded()? => {
                let packages: Vec<_> = packages
                    .iter()
                    .map(|package| FloxPackage::parse(package, &flox.channels, DEFAULT_CHANNEL))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .dedup()
                    .collect();

                let environment =
                    resolve_environment(&flox, environment.as_deref(), "install").await?;

                // todo use set?
                // let installed = environment
                //     .packages(&flox.nix(Default::default()), &flox.system)
                //     .await?
                //     .into_iter()
                //     .map(From::from)
                //     .collect::<Vec<FloxPackage>>();

                // if let Some(installed) = packages.iter().find(|pkg| installed.contains(pkg)) {
                //     anyhow::bail!("{installed} is already installed");
                // }

                let mut environment = environment
                    .modify_in(tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path())
                    .await
                    .context("Could not make modifyable copy of environment")?;

                environment
                    .install(packages, &flox.nix(Default::default()), &flox.system)
                    .await
                    .context("could not install packages")?;

                environment.finish().context("Could not apply changes")?;
            },

            EnvironmentCommands::Remove {
                environment_args: _,
                environment,
                packages,
            } => {
                let packages: Vec<_> = packages
                    .iter()
                    .map(|package| FloxPackage::parse(package, &flox.channels, DEFAULT_CHANNEL))
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .dedup()
                    .collect();

                let environment =
                    resolve_environment(&flox, environment.as_deref(), "install").await?;
                let mut environment = environment
                    .modify_in(tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path())
                    .await
                    .context("Could not make modifyable copy of environment")?;

                environment
                    .uninstall(packages, &flox.nix(Default::default()), &flox.system)
                    .await
                    .context("could not uninstall packages")?;

                environment.finish().context("Could not apply changes")?;
            },

            EnvironmentCommands::WipeHistory {
                // TODO use environment_args.system?
                environment_args: _,
                environment,
            } => {
                let environment_name = environment.as_deref();
                let environment_ref: environment_ref::EnvironmentRef =
                    resolve_environment_ref(&flox, "wipe-history", environment_name).await?;

                let env = environment_ref.to_env().context("Environment not found")?;

                if env.delete_symlinks()? {
                    // The flox nix instance is created with `--quiet --quiet`
                    // because nix logs are passed to stderr unfiltered.
                    // nix store gc logs are more useful,
                    // thus we use 3 `--verbose` to have them appear.
                    let nix = flox.nix::<NixCommandLine>(vec![
                        "--verbose".to_string(),
                        "--verbose".to_string(),
                        "--verbose".to_string(),
                    ]);
                    let store_gc_command = StoreGc {
                        ..StoreGc::default()
                    };

                    info!("Running garbage collection. This may take a while...");
                    store_gc_command.run(&nix, &Default::default()).await?;
                } else {
                    info!("No old generations found to clean up.")
                }
            },

            _ => flox_forward(&flox).await?,
        }

        Ok(())
    }
}

async fn resolve_environment<'flox>(
    flox: &'flox Flox,
    environment_name: Option<&str>,
    subcommand: &str,
) -> Result<Environment<Read>, anyhow::Error> {
    let environment_ref = resolve_environment_ref(flox, subcommand, environment_name).await?;
    let environment = environment_ref
        .to_env()
        .context("Could not use environment")?;
    Ok(environment)
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
    /// * in current shell: eval "$(flox activate)"
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

        /// Replace environment declaration with that in FILE
        #[bpaf(long, short, argument("FILE"))]
        file: Option<PathBuf>,
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

    /// delete builds of non-current versions of an environment
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
