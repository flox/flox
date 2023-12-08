use std::env;
use std::fs::{self, File};
use std::io::{stdin, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use crossterm::{cursor, QueueableCommand};
use flox_rust_sdk::flox::{Auth0Client, EnvironmentName, EnvironmentOwner, EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
};
use flox_rust_sdk::models::environment::path_environment::{self, Original, PathEnvironment};
use flox_rust_sdk::models::environment::{
    EditResult,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    UninitializedEnvironment,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
    FLOX_ENV_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR,
};
use flox_rust_sdk::models::floxmetav2::FloxmetaV2Error;
use flox_rust_sdk::models::manifest::list_packages;
use flox_rust_sdk::nix::command::StoreGc;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use indoc::indoc;
use itertools::Itertools;
use log::{debug, error, info};
use tempfile::NamedTempFile;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{activated_environments, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
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
            .detect_concrete_environment(&flox, "edit")?
            .into_dyn_environment();

        let result = match self.provided_manifest_contents()? {
            // If provided with the contents of a manifest file, either via a path to a file or via
            // contents piped to stdin, use those contents to try building the environment.
            Some(new_manifest) => environment.edit(&flox, new_manifest).await?,
            // If not provided with new manifest contents, let the user edit the file directly
            // via $EDITOR or $VISUAL (as long as `flox edit` was invoked interactively).
            None => self.interactive_edit(flox, environment.as_mut()).await?,
        };
        match result {
            EditResult::Unchanged => {
                println!("⚠️  no changes made to environment");
            },
            EditResult::ReActivateRequired => {
                if activated_environments().contains(&environment.parent_path()?) {
                    println!(indoc::indoc! {"
                            Your manifest has changes that cannot be automatically applied to your current environment.

                            Please `exit` the environment and run `flox activate` to see these changes."});
                } else {
                    println!("✅ environment successfully edited");
                }
            },
            EditResult::Success => {
                println!("✅ environment successfully edited");
            },
        }
        Ok(())
    }

    /// Interactively edit the manifest file
    async fn interactive_edit(
        &self,
        flox: Flox,
        environment: &mut dyn Environment,
    ) -> Result<EditResult> {
        if !Dialog::can_prompt() {
            bail!("Can't edit interactively in non-interactive context")
        }

        let editor = Self::determine_editor()?;

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
            match environment.edit(&flox, new_manifest).await {
                Err(e) => {
                    error!("Environment invalid; building resulted in an error: {e}");
                    if !Dialog::can_prompt() {
                        bail!("Can't prompt to continue editing in non-interactive context");
                    }
                    if !should_continue.clone().prompt().await? {
                        bail!("Environment editing cancelled");
                    }
                },
                Ok(result) => {
                    return Ok(result);
                },
            }
        }
    }

    /// Determines the editor to use for interactive editing
    ///
    /// If $EDITOR or $VISUAL is set, use that. Otherwise, try to find a known editor in $PATH.
    /// The known editor selected is the first one found in $PATH from the following list:
    ///
    ///   vim, vi, nano, emacs.
    fn determine_editor() -> Result<PathBuf> {
        let editor = std::env::var("EDITOR").or(std::env::var("VISUAL")).ok();

        if let Some(editor) = editor {
            return Ok(PathBuf::from(editor));
        }

        let path_var = env::var("PATH").context("$PATH not set")?;

        let (path, editor) = env::split_paths(&path_var)
            .cartesian_product(["vim", "vi", "nano", "emacs"])
            .find(|(path, editor)| path.join(editor).exists())
            .context("no known editor found in $PATH")?;

        debug!("Using editor {:?} from {:?}", editor, path);

        Ok(path.join(editor))
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
    fn edited_manifest_contents(
        path: impl AsRef<Path>,
        editor: impl AsRef<Path>,
    ) -> Result<String> {
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
        match self
            .environment
            .detect_concrete_environment(&flox, "delete")?
        {
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

        let mut stderr = std::io::stdout();
        stderr
            .queue(cursor::SavePosition)
            .context("couldn't set cursor positon")?;
        stderr
            .write_all("Building environment...\n".as_bytes())
            .context("could't write progress message")?;
        stderr.flush().context("could't flush stderr")?;

        let activation_path = environment.activation_path(&flox).await?;

        stderr
            .queue(cursor::RestorePosition)
            .context("couldn't restore cursor position")?;
        stderr.flush().context("could't flush stderr")?;

        // We don't have access to the current PS1 (it's not exported), so we
        // can't modify it. Instead set FLOX_PROMPT_ENVIRONMENTS and let the
        // activation script set PS1 based on that.
        let flox_prompt_environments = env::var(FLOX_PROMPT_ENVIRONMENTS_VAR)
            .map_or(prompt_name.clone(), |prompt_environments| {
                format!("{prompt_name} {prompt_environments}")
            });

        // Add to FLOX_ACTIVE_ENVIRONMENTS so we can detect what environments are active.
        let parent_path = environment.parent_path()?;
        let mut active_environments = vec![parent_path];
        if let Ok(existing_environments) = env::var(FLOX_ACTIVE_ENVIRONMENTS_VAR) {
            active_environments.extend(env::split_paths(&existing_environments));
        };
        let flox_active_environments = env::join_paths(active_environments).context(
            "Cannot activate environment because its path contains an invalid character",
        )?;

        // TODO more sophisticated detection?
        let shell = if let Ok(shell) = env::var("SHELL") {
            shell
        } else {
            bail!("SHELL must be set");
        };
        let mut command = Command::new(&shell);
        command
            .env(FLOX_PROMPT_ENVIRONMENTS_VAR, flox_prompt_environments)
            .env(FLOX_ENV_VAR, &activation_path)
            .env(FLOX_ACTIVE_ENVIRONMENTS_VAR, flox_active_environments)
            .env(
                "FLOX_PROMPT_COLOR_1",
                // default to SlateBlue3
                env::var("FLOX_PROMPT_COLOR_1").unwrap_or("61".to_string()),
            )
            .env(
                "FLOX_PROMPT_COLOR_2",
                // default to LightSalmon1
                env::var("FLOX_PROMPT_COLOR_2").unwrap_or("216".to_string()),
            );

        if shell.ends_with("bash") {
            command
                .arg("--rcfile")
                .arg(activation_path.join("activate").join("bash"));
        } else if shell.ends_with("zsh") {
            // From man zsh:
            // Commands are then read from $ZDOTDIR/.zshenv.  If the shell is a
            // login shell, commands are read from /etc/zprofile and then
            // $ZDOTDIR/.zprofile.  Then, if the shell is interactive, commands
            // are read from /etc/zshrc and then $ZDOTDIR/.zshrc.  Finally, if
            // the shell is a login shell, /etc/zlogin and $ZDOTDIR/.zlogin are
            // read.
            //
            // We want to add our customizations as late as possible in the
            // initialization process - if, e.g. the user has prompt
            // customizations, we want ours to go last. So we put our
            // customizations at the end of .zshrc, passing our customizations
            // using FLOX_ZSH_INIT_SCRIPT.
            // Otherwise, we want initialization to proceed as normal, so the
            // files in our ZDOTDIR source global rcs and user rcs.
            // We disable global rc files and instead source them manually so we
            // can control the ZDOTDIR they are run with - this is important
            // since macOS sets
            // HISTFILE=${ZDOTDIR:-$HOME}/.zsh_history
            // in /etc/zshrc.
            if let Ok(zdotdir) = env::var("ZDOTDIR") {
                command.env("FLOX_ORIG_ZDOTDIR", zdotdir);
            }
            command
                .env("ZDOTDIR", env!("FLOX_ZDOTDIR"))
                .env(
                    "FLOX_ZSH_INIT_SCRIPT",
                    activation_path.join("activate").join("zsh"),
                )
                .arg("--no-globalrcs");
        } else {
            bail!("Unsupported SHELL '{shell}'");
        };

        debug!("running activation command: {:?}", command);
        let error = command.exec();

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
    #[bpaf(long("name"), short('n'), argument("name"))]
    env_name: Option<EnvironmentName>,
}

impl Init {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("init");

        let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

        let home_dir = dirs::home_dir().unwrap();

        let env_name = if let Some(name) = self.env_name {
            name
        } else if dir == home_dir {
            "default".parse()?
        } else {
            dir.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .context("Can't init in root")?
                .parse()?
        };

        let env = PathEnvironment::<Original>::init(
            PathPointer::new(env_name),
            &dir,
            flox.temp_dir.clone(),
        )?;

        println!(
            indoc::indoc! {"
            ✨ Created environment {name} ({system})

            Next:
              $ flox search <package>    <- Search for a package
              $ flox install <package>   <- Install a package into an environment
              $ flox activate            <- Enter the environment
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
            .detect_concrete_environment(&flox, "list using")?
            .into_dyn_environment();

        let manifest_contents = env.manifest_content()?;
        if let Some(pkgs) = list_packages(&manifest_contents)? {
            pkgs.iter().for_each(|pkg| println!("{}", pkg));
        }
        Ok(())
    }
}

fn environment_description(environment: &ConcreteEnvironment) -> Result<String, EnvironmentError2> {
    Ok(match environment {
        ConcreteEnvironment::Managed(environment) => {
            format!(
                "{}/{} at {}",
                environment.owner(),
                environment.name(),
                environment.path.to_string_lossy()
            )
        },
        ConcreteEnvironment::Path(environment) => {
            format!(
                "{} at {}",
                environment.name(),
                environment.parent_path()?.to_string_lossy()
            )
        },
        _ => todo!(),
    })
}

/// Generate a description for an environment that has not yet been opened.
///
/// TODO: we should share this implementation with environment_description().
pub fn hacky_environment_description(
    uninitialized: &UninitializedEnvironment,
) -> Result<String, EnvironmentError2> {
    Ok(match &uninitialized.pointer {
        EnvironmentPointer::Managed(managed_pointer) => {
            format!(
                "{}/{} at {}",
                managed_pointer.owner,
                managed_pointer.name,
                uninitialized.path.to_string_lossy(),
            )
        },
        EnvironmentPointer::Path(path_pointer) => {
            format!(
                "{} at {}",
                path_pointer.name,
                uninitialized.path.to_string_lossy()
            )
        },
    })
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
        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "install to")?;
        let description = environment_description(&concrete_environment)?;
        let mut environment = concrete_environment.into_dyn_environment();
        let installation = environment.install(self.packages.clone(), &flox).await?;
        if installation.new_manifest.is_some() {
            // Print which new packages were installed
            for pkg in self.packages.iter() {
                if let Some(false) = installation.already_installed.get(pkg) {
                    info!("✅ '{pkg}' installed to environment {description}");
                } else {
                    info!("⚠️ '{pkg}' already installed to environment {description}");
                }
            }
        } else {
            info!("⚠️ package(s) already installed to environment {description}");
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
        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "uninstall from")?;
        let description = environment_description(&concrete_environment)?;
        let mut environment = concrete_environment.into_dyn_environment();
        let _ = environment.uninstall(self.packages.clone(), &flox).await?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        self.packages
            .iter()
            .for_each(|p| info!("🗑️  '{p}' uninstalled from environment {description}"));
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
            .detect_concrete_environment(&flox, "wipe history of")?
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
    /// Directory to push the environment from (default: current directory)
    dir: Option<PathBuf>,

    /// Owner to push push environment to (default: current user)
    #[bpaf(long, short)]
    owner: Option<EnvironmentOwner>,

    /// forceably overwrite the remote copy of the environment
    #[bpaf(long, short)]
    force: bool,
}

impl Push {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("push");
        let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

        match EnvironmentPointer::open(&dir)? {
            EnvironmentPointer::Managed(managed_pointer) => {
                if self.owner.is_some() {
                    bail!("Environment already linked to a remote")
                }

                Self::push_managed_env(&flox, managed_pointer, dir, self.force)
                    .context("Could not push managed environment")?;
            },
            EnvironmentPointer::Path(path_pointer) => {
                let owner = if let Some(owner) = self.owner {
                    owner
                } else {
                    let base_url = std::env::var("FLOX_OAUTH_BASE_URL")
                        .unwrap_or(env!("OAUTH_BASE_URL").to_string());
                    let client = Auth0Client::new(
                        base_url,
                        flox.floxhub_token.clone().context("Need to be logged in")?,
                    );
                    let user_name = client
                        .get_username()
                        .await
                        .context("Could not get username from github")?;
                    user_name
                        .parse::<EnvironmentOwner>()
                        .context("Invalid owner name")?
                };
                Self::push_make_managed(&flox, path_pointer, &dir, owner, self.force)
                    .context("Could not push new environment")?;
            },
        }
        Ok(())
    }

    fn push_managed_env(
        flox: &Flox,
        managed_pointer: ManagedPointer,
        dir: PathBuf,
        force: bool,
    ) -> Result<()> {
        let mut env = ManagedEnvironment::open(flox, managed_pointer, dir.join(DOT_FLOX))
            .context("Could not open environment")?;
        env.push(force).context("Could not push environment")?;

        Ok(())
    }

    /// pushes a path environment in a directory to floxhub and makes it a managed environment
    fn push_make_managed(
        flox: &Flox,
        path_pointer: PathPointer,
        dir: &Path,
        owner: EnvironmentOwner,
        force: bool,
    ) -> Result<()> {
        let dot_flox_path = dir.join(DOT_FLOX);
        let path_environment =
            path_environment::PathEnvironment::open(path_pointer, dot_flox_path, &flox.temp_dir)?;

        ManagedEnvironment::push_new(
            flox,
            path_environment,
            owner.parse().unwrap(),
            &flox.temp_dir,
            force,
        )
        .map_err(Self::convert_error)?;

        Ok(())
    }

    fn convert_error(err: ManagedEnvironmentError) -> anyhow::Error {
        if let ManagedEnvironmentError::OpenFloxmeta(FloxmetaV2Error::LoggedOut) = err {
            anyhow!(indoc! {"
                Could not push environment: not logged in to floxhub.

                Please login to floxhub with `flox auth login`
                "})
        } else {
            anyhow!(err)
        }
    }
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
        /// Directory containing a managed environment to pull
        dir: Option<PathBuf>,
        /// forceably overwrite the local copy of the environment
        #[bpaf(long, short)]
        force: bool,
    },
}

impl Default for PullSelect {
    fn default() -> Self {
        PullSelect::Existing {
            dir: Default::default(),
            force: Default::default(),
        }
    }
}

/// Pull environment from flox hub
#[derive(Bpaf, Clone)]
pub struct Pull {
    #[bpaf(external(pull_select), fallback(Default::default()))]
    pull_select: PullSelect,
}

impl Pull {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("pull");
        match self.pull_select {
            PullSelect::New { dir, remote } => {
                let dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull {remote:?} into {dir:?}");

                Self::pull_new_environment(&flox, dir.join(DOT_FLOX), remote)?;
            },
            PullSelect::Existing { dir, force } => {
                let dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull changes for environment found in {dir:?}");

                let pointer = {
                    let p = EnvironmentPointer::open(&dir)
                        .with_context(|| format!("No environment found in {dir:?}"))?;
                    match p {
                        EnvironmentPointer::Managed(managed_pointer) => managed_pointer,
                        EnvironmentPointer::Path(_) => bail!("Cannot pull into a path environment"),
                    }
                };

                Self::pull_existing_environment(&flox, dir.join(DOT_FLOX), pointer, force)?;
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
        force: bool,
    ) -> Result<()> {
        let mut env = ManagedEnvironment::open(flox, pointer, dot_flox_path)
            .context("Could not open environment")?;
        env.pull(force).context("Could not pull environment")?;

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

        let result =
            ManagedEnvironment::open(flox, pointer, &dot_flox_path).map_err(Self::convert_error);
        if let Err(err) = result {
            fs::remove_dir_all(dot_flox_path).context("Could not clean up .flox/ directory")?;
            Err(err)?;
        }
        Ok(())
    }

    fn convert_error(err: EnvironmentError2) -> anyhow::Error {
        if let EnvironmentError2::ManagedEnvironment(ManagedEnvironmentError::OpenFloxmeta(
            FloxmetaV2Error::LoggedOut,
        )) = err
        {
            anyhow!(indoc! {"
                Could not pull environment: not logged in to floxhub.

                Please login to floxhub with `flox auth login`
                "})
        } else {
            anyhow!(err)
        }
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
