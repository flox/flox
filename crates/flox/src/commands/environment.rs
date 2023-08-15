use std::env::current_dir;
use std::fs::File;
use std::io::stdin;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use bpaf::{construct, Bpaf, Parser, ShellComp};
use flox_rust_sdk::flox::{EnvironmentName, Flox};
use flox_rust_sdk::models::environment::{Environment, Read};
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::command::{Shell, StoreGc};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::flox_package::FloxPackage;
use flox_types::constants::{DEFAULT_CHANNEL, LATEST_VERSION};
use itertools::Itertools;
use log::{error, info};

use crate::config::features::Feature;
use crate::utils::dialog::{Confirm, Dialog};
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
            EnvironmentCommands::Init { .. } => subcommand_metric!("init"),
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
            EnvironmentCommands::Uninstall { .. } => subcommand_metric!("remove"),
            EnvironmentCommands::Rollback { .. } => subcommand_metric!("rollback"),
            EnvironmentCommands::SwitchGeneration { .. } => subcommand_metric!("switch"),
            EnvironmentCommands::Upgrade { .. } => subcommand_metric!("upgrade"),
            EnvironmentCommands::WipeHistory { .. } => subcommand_metric!("wipe-history"),
        }

        match self {
            EnvironmentCommands::Edit {
                environment_args: _,
                environment,
                file,
            } if !Feature::Env.is_forwarded()? => 'edit: {
                let environment =
                    resolve_environment(&flox, environment.as_deref(), "install").await?;
                let mut environment = environment
                    .modify_in(tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path())
                    .await?;

                let nix = flox.nix(Default::default());

                if let Some(file) = file {
                    let file: Box<dyn std::io::Read> = if file == Path::new("-") {
                        Box::new(stdin())
                    } else {
                        Box::new(File::open(file).unwrap())
                    };

                    environment
                        .set_environment(file, &nix, &flox.system)
                        .await?;
                    break 'edit;
                }

                let editor = std::env::var("EDITOR").context("$EDITOR not set")?;
                if !Dialog::can_prompt() {
                    bail!("Can't open editor in non interactive context");
                }

                loop {
                    let path = environment.flox_nix_path();
                    let mut command = Command::new(&editor);
                    command.arg(&path);

                    let child = command.spawn().context("editor command failed")?;
                    let _ = child.wait_with_output().context("editor command failed")?;

                    match environment
                        .set_environment(
                            std::fs::read_to_string(&path).unwrap().as_bytes(),
                            &nix,
                            &flox.system,
                        )
                        .await
                    {
                        Ok(_) => {
                            break;
                        },
                        Err(e) => {
                            error!("Environment invalid, building resulted in an error: {e}");
                            let again = Dialog {
                                message: "Continue editing?",
                                help_message: Default::default(),
                                typed: Confirm {
                                    default: Some(true),
                                },
                            }
                            .prompt()
                            .await?;
                            if !again {
                                break 'edit;
                            }
                        },
                    };
                }
                environment
                    .finish()
                    .context("Failed applying environemnt changes")?;
            },

            EnvironmentCommands::Destroy { environment, .. } if !Feature::Env.is_forwarded()? => {
                let environment =
                    resolve_environment(&flox, environment.as_deref(), "install").await?;

                environment
                    .delete()
                    .context("Failed to delete environment")?;
            },

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
                        ..Default::default()
                    },
                    installables: [environment.flake_attribute(&flox.system).into()].into(),
                    ..Default::default()
                };

                let nix = flox.nix(Default::default());
                command.run(&nix, &Default::default()).await?
            },

            EnvironmentCommands::Init {
                environment_args: _,
                environment,
            } => {
                let current_dir = std::env::current_dir().unwrap();
                let home_dir = dirs::home_dir().unwrap();

                let name = if let Some(name) = environment.clone() {
                    name
                } else if current_dir == home_dir {
                    "default".to_string()
                } else {
                    current_dir
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .context("Can't init in root")?
                };

                let name = EnvironmentName::from_str(&name)?;

                let env = Environment::init(&current_dir, name).await?;

                println!(
                    indoc::indoc! {"
                    ✨ created environment {name} ({system})

                    Enter the environment with \"flox activate\"
                    Search and install packages with \"flox search {{packagename}}\" and \"flox install {{packagename}}\"
                    "},
                    name = env.environment_ref(),
                    system = flox.system
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

                println!("Packages in {}:", env.environment_ref());
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
                let env = Environment::discover(std::env::current_dir().unwrap())?;

                if let Some(env) = env {
                    println!("{}", env.environment_ref());
                } else {
                    println!();
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

            EnvironmentCommands::Uninstall {
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

                let env = Environment::open(current_dir().unwrap(), environment_ref)
                    .context("Environment not found")?;

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
    #[bpaf(command, long("create"))]
    Init {
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
    #[bpaf(command, long("remove"), long("rm"))]
    Uninstall {
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
