use std::env;
use std::fs::{self, File};
use std::io::stdin;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::{EnvironmentName, EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::path_environment::{Original, PathEnvironment};
use flox_rust_sdk::models::environment::remote_environment::RemoteEnvironment;
use flox_rust_sdk::models::environment::{
    Environment,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
};
use flox_rust_sdk::models::environment_ref;
use flox_rust_sdk::models::manifest::list_packages;
use flox_rust_sdk::nix::command::StoreGc;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use log::{debug, error, info};
use tempfile::NamedTempFile;

use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
}

#[derive(Debug, Bpaf, Clone)]
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
            EnvironmentSelect::Dir(path) => {
                let pointer = EnvironmentPointer::open(path)
                    .with_context(|| format!("No environment found in {path:?}"))?;

                match pointer {
                    EnvironmentPointer::Path(path_pointer) => {
                        let dot_flox_path = path.join(DOT_FLOX);
                        ConcreteEnvironment::Path(PathEnvironment::open(
                            path_pointer,
                            dot_flox_path,
                            &flox.temp_dir,
                        )?)
                    },
                    EnvironmentPointer::Managed(managed_pointer) => {
                        let env = ManagedEnvironment::open(flox, managed_pointer, path)?;
                        ConcreteEnvironment::Managed(env)
                    },
                }
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

/// Activate an environment
///
/// When called with no arguments `flox activate` will look for a `.flox` directory
/// in the current directory. Calling `flox activate` in your home directory will
/// activate a default environment. Environments in other directories and remote
/// environments are activated with the `-d` and `-r` flags respectively.
#[derive(Bpaf, Clone)]
pub struct Activate {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Activate {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("activate");

        let concrete_environment = self.environment.to_concrete_environment(&flox)?;

        // TODO could move this to a pretty print method on the Environment trait?
        let prompt_name = match concrete_environment {
            // Note that the same environment could show up twice without any
            // indication of which comes from which path
            ConcreteEnvironment::Managed(ref managed) => {
                format!("{}/{}", managed.owner(), managed.name())
            },
            ConcreteEnvironment::Path(ref path) => path.name().to_string(),
            _ => todo!(),
        };

        let mut environment = concrete_environment.into_dyn_environment();

        let nix = flox.nix(Default::default());
        let activation_path = environment.activation_path(&flox, &nix).await?;

        let flox_prompt_environments = env::var("FLOX_PROMPT_ENVIRONMENTS")
            .map_or(prompt_name.clone(), |prompt_environments| {
                format!("{prompt_environments} {prompt_name}")
            });

        // We don't have access to the current PS1 (it's not exported), so we
        // can't modify it. Instead set FLOX_PROMPT_ENVIRONMENTS and let the
        // activation script set PS1 based on that.
        let error = Command::new(activation_path.join("activate"))
            .env("FLOX_PROMPT_ENVIRONMENTS", flox_prompt_environments)
            .env("FLOX_ENV", activation_path)
            .exec();

        // exec should never return

        bail!("Failed to exec subshell: {error}");
    }
}

/// Create an environment in the current directory
#[derive(Bpaf, Clone)]
pub struct Init {
    #[allow(dead_code)] // pending spec for `-e`, `--dir` behaviour
    #[bpaf(external(environment_args), group_help("Environment Options"))]
    environment_args: EnvironmentArgs,

    /// Directory to create the environment in (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Name of the environment
    ///
    /// "$(basename $PWD)" or "default" if in $HOME
    #[bpaf(long, short, argument("name"))]
    name: Option<EnvironmentName>,
}

impl Init {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init");

        let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

        let home_dir = dirs::home_dir().unwrap();

        let name = if let Some(name) = self.name {
            name
        } else if dir == home_dir {
            "default".parse()?
        } else {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?
                .parse()?
        };

        let env =
            PathEnvironment::<Original>::init(PathPointer::new(name), &dir, flox.temp_dir.clone())?;

        println!(
            indoc::indoc! {"
            ✨ created environment {name} ({system})

            Enter the environment with \"flox activate\"
            Search and install packages with \"flox search {{packagename}}\" and \"flox install {{packagename}}\"
            "},
            name = env.name(),
            system = flox.system
        );
        Ok(())
    }
}

/// List packages installed in an environment
#[derive(Bpaf, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl List {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("list");

        let env = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();

        let manifest_contents = env.manifest_content()?;
        if let Some(pkgs) = list_packages(&manifest_contents)? {
            pkgs.iter().for_each(|pkg| println!("{}", pkg));
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

        debug!(
            "installing packages [{}] to {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );
        let mut environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();
        let nix = flox.nix::<NixCommandLine>(vec![]);
        let installation = environment
            .install(self.packages.clone(), &nix, flox.system.clone())
            .await?;
        if installation.new_manifest.is_some() {
            // Print which new packages were installed
            for pkg in self.packages.iter() {
                if let Some(false) = installation.already_installed.get(pkg) {
                    info!("✅ '{pkg}' installed to environment");
                } else {
                    info!("🛑 '{pkg}' already installed");
                }
            }
        } else {
            info!("🛑 package(s) already installed");
        }
        Ok(())
    }
}

/// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(positional("PACKAGES"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Uninstall {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("uninstall");

        debug!(
            "uninstalling packages [{}] from {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );
        let mut environment = self
            .environment
            .to_concrete_environment(&flox)?
            .into_dyn_environment();
        let nix = flox.nix::<NixCommandLine>(vec![]);
        let _ = environment
            .uninstall(self.packages.clone(), &nix, flox.system.clone())
            .await?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        self.packages
            .iter()
            .for_each(|p| info!("🗑️  '{p}' uninstalled from environment"));
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("generations");

        todo!("this command is planned for a future release")
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("history");

        todo!("this command is planned for a future release")
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("push");

        todo!("this command is planned for a future release")
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

#[derive(Debug, Clone, Bpaf)]
enum PullSelect {
    New {
        /// Directory to create the environment in (default: current directory)
        dir: Option<PathBuf>,
        /// ID of the environment to pull
        remote: EnvironmentRef,
    },
    Existing {
        dir: Option<PathBuf>,
    },
}

impl Default for PullSelect {
    fn default() -> Self {
        PullSelect::Existing {
            dir: Default::default(),
        }
    }
}

/// Pull environment from flox hub
#[derive(Bpaf, Clone)]
pub struct Pull {
    #[bpaf(external(pull_select), fallback(Default::default()))]
    pull_select: PullSelect,

    /// forceably overwrite the local copy of the environment
    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(long, short)]
    force: bool,
}

impl Pull {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("pull");
        match self.pull_select {
            PullSelect::New { dir, remote } => {
                let dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());
                Self::pull_new_environment(&flox, dir.join(DOT_FLOX), remote)?;
            },
            PullSelect::Existing { dir } => {
                let dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());
                let pointer = {
                    let p = EnvironmentPointer::open(dir.join(DOT_FLOX))
                        .with_context(|| format!("No environment found in {dir:?}"))?;
                    match p {
                        EnvironmentPointer::Managed(managed_pointer) => managed_pointer,
                        EnvironmentPointer::Path(_) => bail!("Cannot pull into a path environment"),
                    }
                };

                Self::pull_existing_environment(&flox, dir.join(DOT_FLOX), pointer)?;
            },
        }

        Ok(())
    }

    /// Update an existing environment with the latest version from floxhub
    ///
    /// Opens the environment and calls [ManagedEnvironment::pull] on it,
    /// which will update the lockfile.
    fn pull_existing_environment(
        flox: &Flox,
        dot_flox_path: PathBuf,
        pointer: ManagedPointer,
    ) -> Result<()> {
        let mut env = ManagedEnvironment::open(flox, pointer, dot_flox_path)
            .context("Could not initialize environment")?;
        env.pull().context("Could not pull environment")?;

        Ok(())
    }

    /// Pull a new environment from floxhub into the given directory
    ///
    /// This will create a new environment in the given directory.
    /// Uses [ManagedEnvironment::open] which will try to clone the environment.
    ///
    /// If the directory already exists, this will fail early.
    /// If opening the environment fails, the .flox/ directory will be cleaned up.
    fn pull_new_environment(
        flox: &Flox,
        dot_flox_path: PathBuf,
        env_ref: EnvironmentRef,
    ) -> Result<()> {
        if dot_flox_path.exists() {
            bail!("Cannot pull a new environment into an existing one")
        }
        let pointer = ManagedPointer::from(env_ref);

        let pointer_content =
            serde_json::to_string_pretty(&pointer).context("Could not serialize pointer")?;
        let pointer_path = dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME);

        fs::create_dir_all(&dot_flox_path).context("Could not create .flox/ directory")?;
        fs::write(pointer_path, pointer_content).context("Could not write pointer")?;

        let result = ManagedEnvironment::open(flox, pointer, &dot_flox_path)
            .context("Could not initialize environment");
        if let Err(err) = result {
            fs::remove_dir_all(dot_flox_path).context("Could not clean up .flox/ directory")?;
            Err(err)?;
        }
        Ok(())
    }
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("rollback");

        todo!("this command is planned for a future release")
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("switch-generation");

        todo!("this command is planned for a future release")
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
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");

        todo!("this command is planned for a future release")
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[allow(unused)]
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}
impl Containerize {
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        todo!("this command is planned for a future release");
    }
}
