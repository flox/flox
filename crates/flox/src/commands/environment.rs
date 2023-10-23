use std::fs::File;
use std::io::stdin;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bpaf::{construct, Bpaf, Parser, ShellComp};
use flox_rust_sdk::flox::{EnvironmentName, Flox};
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::path_environment::{Original, PathEnvironment};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    DOT_FLOX,
};
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::nix::arguments::eval::EvaluationArgs;
use flox_rust_sdk::nix::command::{Shell, StoreGc};
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use flox_rust_sdk::prelude::flox_package::FloxPackage;
use flox_types::constants::{DEFAULT_CHANNEL, LATEST_VERSION};
use itertools::Itertools;
use log::{error, info};
use tempfile::NamedTempFile;

use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::display::packages_to_string;
use crate::{flox_forward, subcommand_metric};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
}

pub type EnvironmentRef = String;

#[derive(Bpaf, Clone)]
pub enum EnvironmentSelect {
    Dir(
        /// Path containing a .flox/ directory
        #[bpaf(long("dir"), short('d'), argument("path"))]
        PathBuf,
    ),
    Remote(
        /// A remote environment on floxhub
        #[bpaf(long("remote"), short('r'), argument("owner/name"))]
        environment_ref::EnvironmentRef,
    ),
}

impl Default for EnvironmentSelect {
    fn default() -> Self {
        EnvironmentSelect::Dir(PathBuf::from("./"))
    }
}

impl EnvironmentSelect {
    fn to_concrete_environment(&self, flox: &Flox) -> Result<ConcreteEnvironment> {
        let env = match self {
            EnvironmentSelect::Dir(path) => match EnvironmentPointer::open(path)? {
                EnvironmentPointer::Path(_path_pointer) => {
                    let dot_flox_path = path.join(DOT_FLOX);
                    ConcreteEnvironment::Path(PathEnvironment::open(dot_flox_path, &flox.temp_dir)?)
                },
                EnvironmentPointer::Managed(managed_pointer) => ConcreteEnvironment::Managed(
                    ManagedEnvironment::open(flox, managed_pointer, path)?,
                ),
            },
            EnvironmentSelect::Remote(_) => todo!(),
        };

        Ok(env)
    }
}

enum ConcreteEnvironment {
    /// Container for [PathEnvironment]
    Path(PathEnvironment<Original>),
    /// Container for [ManagedEnvironment]
    #[allow(unused)] // pending implementation of ManagedEnvironment
    Managed(ManagedEnvironment),
    /// Container for [RemoteEnvironment]
    #[allow(unused)] // pending implementation of RemoteEnvironment
    Remote(RemoteEnvironment),
}

impl ConcreteEnvironment {
    fn into_dyn_environment(self) -> Box<dyn Environment> {
        match self {
            ConcreteEnvironment::Path(path_env) => Box::new(path_env),
            ConcreteEnvironment::Managed(managed_env) => Box::new(managed_env),
            ConcreteEnvironment::Remote(remote_env) => Box::new(remote_env),
        }
    }
}

/// Edit declarative environment configuration
#[derive(Bpaf, Clone)]
pub struct Edit {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Replace environment declaration with that in FILE
    #[bpaf(long, short, argument("FILE"))]
    file: Option<PathBuf>,
}

impl Edit {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("edit");

        let mut environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();
        let nix = flox.nix(Default::default());

        match self.provided_manifest_contents()? {
            // If provided with the contents of a manifest file, either via a path to a file or via
            // contents piped to stdin, use those contents to try building the environment.
            Some(new_manifest) => {
                environment
                    .edit(&nix, flox.system.clone(), new_manifest)
                    .await?;
                Ok(())
            },
            // If not provided with new manifest contents, let the user edit the file directly
            // via $EDITOR or $VISUAL (as long as `flox edit` was invoked interactively).
            None => {
                let editor = std::env::var("EDITOR")
                    .or(std::env::var("VISUAL"))
                    .context("no editor found; neither EDITOR nor VISUAL are set")?;
                // TODO: check for interactivity before allowing the editor to be opened
                // Make a copy of the manifest for the user to edit so failed edits aren't left in
                // the original manifest. You can't put creation/cleanup inside the `edited_manifest_contents`
                // method because the temporary manifest needs to stick around in case the user wants
                // or needs to make successive edits without starting over each time.
                let tmp_manifest = NamedTempFile::new_in(&flox.temp_dir)?;
                std::fs::write(&tmp_manifest, environment.manifest_content()?)?;
                let should_continue = Dialog {
                    message: "Continue editing?",
                    help_message: Default::default(),
                    typed: Confirm {
                        default: Some(true),
                    },
                };
                // Let the user keep editing the file until the build succeeds or the user
                // decides to stop.
                loop {
                    let new_manifest = Edit::edited_manifest_contents(&tmp_manifest, &editor)?;
                    if let Err(e) = environment
                        .edit(&nix, flox.system.clone(), new_manifest)
                        .await
                    {
                        error!("Environment invalid; building resulted in an error: {e}");
                        if !Dialog::can_prompt() {
                            bail!("Can't prompt to continue editing in non-interactive context");
                        }
                        if !should_continue.clone().prompt().await? {
                            bail!("Environment editing cancelled");
                        }
                    } else {
                        break;
                    }
                }
                Ok(())
            },
        }
    }

    /// Retrieves the new manifest file contents if a new manifest file was provided
    fn provided_manifest_contents(&self) -> Result<Option<String>> {
        if let Some(ref file) = self.file {
            let mut file: Box<dyn std::io::Read + Send> = if file == Path::new("-") {
                Box::new(stdin())
            } else {
                Box::new(File::open(file).unwrap())
            };

            let mut contents = String::new();
            file.read_to_string(&mut contents)?;
            Ok(Some(contents))
        } else {
            Ok(None)
        }
    }

    /// Gets a new set of manifest contents after a user edits the file
    fn edited_manifest_contents(path: impl AsRef<Path>, editor: impl AsRef<str>) -> Result<String> {
        let mut command = Command::new(editor.as_ref());
        command.arg(path.as_ref());

        let child = command.spawn().context("editor command failed")?;
        let _ = child.wait_with_output().context("editor command failed")?;

        let contents = std::fs::read_to_string(path)?;
        Ok(contents)
    }
}

/// Delete an environment
#[derive(Bpaf, Clone)]
pub struct Delete {
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(short, long)]
    force: bool,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(short, long)]
    origin: bool,

    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Delete {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("delete");
        match self.environment.to_concrete_environment(&flox)? {
            ConcreteEnvironment::Path(environment) => environment.delete()?,
            ConcreteEnvironment::Managed(environment) => environment.delete()?,
            ConcreteEnvironment::Remote(environment) => environment.delete()?,
        }

        Ok(())
    }
}

/// Activate environment
///
///
/// Modes:
///  * in current shell: eval "$(flox activate)"
///  * in subshell: flox activate
///  * for command: flox activate -- <command> <args>
#[derive(Bpaf, Clone)]
pub struct Activate {
    #[allow(dead_code)] // TODO: pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // TODO: not yet handled in impl
    #[bpaf(external(activate_run_args))]
    arguments: Option<(String, Vec<String>)>,
}

impl Activate {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("activate");

        let environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

        let command = Shell {
            eval: EvaluationArgs {
                impure: true.into(),
                ..Default::default()
            },
            installables: [environment.flake_attribute(flox.system.clone()).into()].into(),
            ..Default::default()
        };

        let nix = flox.nix(Default::default());
        command.run(&nix, &Default::default()).await?;
        Ok(())
    }
}

/// Create an environment in the current directory
#[derive(Bpaf, Clone)]
pub struct Init {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(long, short, argument("ENV"), hide)]
    environment: Option<EnvironmentRef>,

    /// Name of the environment
    ///
    /// "$(basename $PWD)" or "default" if in $HOME
    #[bpaf(long, short, argument("name"))]
    name: Option<EnvironmentName>,
}

impl Init {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init");

        if self.environment.is_some() {
            bail!(indoc::indoc! {"
                '--environment', '-e' is deprecated.
                Use '(--name | -n) <name>' to create a named env.
                Use 'flox (push | pull)' to create or download an existing environment.
            "});
        }

        let current_dir = std::env::current_dir().unwrap();
        let home_dir = dirs::home_dir().unwrap();

        let name = if let Some(name) = self.name {
            name
        } else if current_dir == home_dir {
            "default".parse()?
        } else {
            current_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?
                .parse()?
        };

        let env = PathEnvironment::<Original>::init(&current_dir, name, flox.temp_dir.clone())?;

        println!(
            indoc::indoc! {"
            ✨ created environment {name} ({system})

            Enter the environment with \"flox activate\"
            Search and install packages with \"flox search {{packagename}}\" and \"flox install {{packagename}}\"
            "},
            name = env.environment_ref(),
            system = flox.system
        );
        Ok(())
    }
}

/// List packages installed in an environment
#[derive(Bpaf, Clone)]
pub struct List {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external(list_output), optional)]
    json: Option<ListOutput>,

    /// The generation to list, if not specified defaults to the current one
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(positional("GENERATION"))]
    generation: Option<u32>,
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

impl List {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("list");

        let env = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

        let catalog = env
            .catalog(&flox.nix(Default::default()), flox.system)
            .await
            .context("Could not get catalog")?;
        // let installed_store_paths = env.installed_store_paths(&flox).await?;

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
        Ok(())
    }
}

/// list all available environments
/// Aliases:
///   environments, envs
#[derive(Bpaf, Clone)]
pub struct Envs {}
impl Envs {
    /// List all available environments
    /// Currently at most one (in the current directory).
    /// This methods is to be changed or removed eventually.
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("envs");

        let env = EnvironmentPointer::open(std::env::current_dir().unwrap());

        match env {
            Ok(EnvironmentPointer::Path(PathPointer { name, .. })) => println!("{name}"),
            Ok(EnvironmentPointer::Managed(ManagedPointer { name, owner, .. })) => {
                println!("{owner}/{name}",)
            },
            Err(EnvironmentError2::DirectoryNotAnEnv) => println!(),
            Err(e) => bail!(e),
        }
        Ok(())
    }
}

/// Install a package into an environment
#[derive(Bpaf, Clone)]
pub struct Install {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(positional("PACKAGES"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Install {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("install");

        let packages: Vec<_> = self
            .packages
            .iter()
            .map(|package| FloxPackage::parse(package, &flox.channels, DEFAULT_CHANNEL))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .dedup()
            .collect();

        let mut environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

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

        let packages_str = packages_to_string(&packages);
        let plural = packages.len() > 1;

        if environment
            .install(packages, &flox.nix(Default::default()), flox.system)
            .await
            .context("could not install packages")?
        {
            println!(
                "✅ Installed {packages_str} into '{}' environment.",
                environment.environment_ref()
            );
        } else {
            let verb = if plural { "are" } else { "is" };
            println!(
                "No changes; {packages_str} {verb} already installed into '{}' environment.",
                environment.environment_ref()
            );
        }
        Ok(())
    }
}

/// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(positional("PACKAGES"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Uninstall {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("uninstall");

        let packages: Vec<_> = self
            .packages
            .iter()
            .map(|package| FloxPackage::parse(package, &flox.channels, DEFAULT_CHANNEL))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .dedup()
            .collect();

        let mut environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

        let packages_str = packages_to_string(&packages);
        let plural = packages.len() > 1;

        if environment
            .uninstall(packages, &flox.nix(Default::default()), flox.system)
            .await
            .context("could not uninstall packages")?
        {
            info!(
                "🗑️ Uninstalled {packages_str} from '{}' environment.",
                environment.environment_ref()
            );
        } else {
            let verb = if plural { "are" } else { "is" };
            info!(
                "No changes; {packages_str} {verb} not installed in '{}' environment.",
                environment.environment_ref()
            );
        }
        Ok(())
    }
}

/// delete builds of non-current versions of an environment
#[derive(Bpaf, Clone)]
pub struct WipeHistory {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl WipeHistory {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("wipe-history");

        let env = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

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
        Ok(())
    }
}

/// export declarative environment manifest to STDOUT
#[derive(Bpaf, Clone)]
pub struct Export {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Export {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("export");

        flox_forward(&flox).await
    }
}

/// list environment generations with contents
#[derive(Bpaf, Clone)]
pub struct Generations {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(long)]
    json: bool,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Generations {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("generations");

        flox_forward(&flox).await
    }
}

/// access to the git CLI for floxmeta repository
#[derive(Bpaf, Clone)]
pub struct Git {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(any("Git Arguments", Some))]
    git_arguments: Vec<String>,
}

impl Git {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("git");

        flox_forward(&flox).await
    }
}

/// show all versions of an environment
#[derive(Bpaf, Clone)]
pub struct History {
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(long, short)]
    oneline: bool,

    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl History {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("history");

        flox_forward(&flox).await
    }
}

/// import declarative environment manifest from STDIN as new generation
#[derive(Bpaf, Clone)]
pub struct Import {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external(ImportFile::parse), fallback(ImportFile::Stdin))]
    file: ImportFile,
}

impl Import {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("import");

        flox_forward(&flox).await
    }
}

#[derive(Clone)]
pub enum ImportFile {
    Stdin,
    Path(PathBuf),
}

impl ImportFile {
    fn parse() -> impl Parser<ImportFile> {
        let stdin = bpaf::any("STDIN (-)", |t: char| {
            (t == '-').then_some(ImportFile::Stdin)
        })
        .help("Use `-` to read from STDIN")
        .complete(|_| vec![("-", Some("Read from STDIN"))]);
        let path = bpaf::positional("PATH")
            .help("Path to export file")
            .complete_shell(ShellComp::File { mask: None })
            .map(ImportFile::Path);
        construct!([stdin, path])
    }
}

/// Send environment to flox hub
#[derive(Bpaf, Clone)]
pub struct Push {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external(push_floxmain_or_env), optional)]
    target: Option<PushFloxmainOrEnv>,

    /// forceably overwrite the remote copy of the environment
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(long, short)]
    force: bool,
}

impl Push {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("push");

        flox_forward(&flox).await
    }
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

/// Pull environment from flox hub
#[derive(Bpaf, Clone)]
pub struct Pull {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(external(pull_floxmain_or_env), optional)]
    target: Option<PullFloxmainOrEnv>,

    /// forceably overwrite the local copy of the environment
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(long, short)]
    force: bool,
}

impl Pull {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("pull");

        flox_forward(&flox).await
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

/// rollback to the previous generation of an environment
#[derive(Bpaf, Clone)]
pub struct Rollback {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[bpaf(long, short, argument("ENV"))]
    #[allow(dead_code)] // not yet handled in impl
    environment: Option<EnvironmentRef>,

    /// Generation to roll back to.
    ///
    /// If omitted, defaults to the previous generation.
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(argument("GENERATION"))]
    to: Option<u32>,
}
impl Rollback {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("rollback");

        flox_forward(&flox).await
    }
}

/// switch to a specific generation of an environment
#[derive(Bpaf, Clone)]
pub struct SwitchGeneration {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(positional("GENERATION"))]
    generation: u32,
}

impl SwitchGeneration {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("switch-generation");

        flox_forward(&flox).await
    }
}

/// upgrade packages using their most recent flake
#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    #[allow(unused)] // Command currently forwarded
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(positional("PACKAGES"))]
    packages: Vec<String>,
}
impl Upgrade {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");

        flox_forward(&flox).await
    }
}

fn activate_run_args() -> impl Parser<Option<(String, Vec<String>)>> {
    let command = bpaf::positional("COMMAND").strict();
    let args = bpaf::any("ARGUMENTS", Some).many();

    bpaf::construct!(command, args).optional()
}
