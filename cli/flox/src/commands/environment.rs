use std::borrow::Cow;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{stdin, stdout, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;
use std::{env, vec};

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::{EnvironmentName, EnvironmentOwner, EnvironmentRef, Flox};
use flox_rust_sdk::models::environment::managed_environment::{
    ManagedEnvironment,
    ManagedEnvironmentError,
};
use flox_rust_sdk::models::environment::path_environment::{self, PathEnvironment};
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    EditResult,
    Environment,
    EnvironmentError2,
    EnvironmentPointer,
    ManagedPointer,
    PathPointer,
    UpdateResult,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
    FLOX_ENV_DIRS_VAR,
    FLOX_ENV_LIB_DIRS_VAR,
    FLOX_ENV_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR,
};
use flox_rust_sdk::models::floxmetav2::FloxmetaV2Error;
use flox_rust_sdk::models::lockfile::{
    FlakeRef,
    Input,
    InstalledPackage,
    LockedManifest,
    LockedManifestError,
    PackageInfo,
    TypedLockedManifest,
};
use flox_rust_sdk::models::manifest::{self, PackageToInstall};
use flox_rust_sdk::models::pkgdb::{call_pkgdb, CallPkgDbError, PkgDbError, PKGDB_BIN};
use flox_rust_sdk::nix::command::StoreGc;
use flox_rust_sdk::nix::command_line::NixCommandLine;
use flox_rust_sdk::nix::Run;
use indexmap::IndexSet;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use log::{debug, error, info, warn};
use tempfile::NamedTempFile;
use toml_edit::Document;
use url::Url;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{
    activated_environments,
    auth,
    ensure_environment_trust,
    ConcreteEnvironment,
    UninitializedEnvironment,
};
use crate::config::Config;
use crate::utils::dialog::{Confirm, Dialog, Select, Spinner};
use crate::utils::didyoumean::{DidYouMean, InstallSuggestion};
use crate::{subcommand_metric, utils};

#[derive(Bpaf, Clone)]
pub struct EnvironmentArgs {
    #[bpaf(short, long, argument("SYSTEM"))]
    pub system: Option<String>,
}

/// Edit declarative environment configuration
#[derive(Bpaf, Clone)]
pub struct Edit {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(edit_action), fallback(EditAction::EditManifest{file: None}))]
    action: EditAction,
}

/// Edit declarative environment configuration
#[derive(Bpaf, Clone)]
pub enum EditAction {
    EditManifest {
        /// Replace environment declaration with that in <file>
        #[bpaf(long, short, argument("file"))]
        file: Option<PathBuf>,
    },

    Rename {
        /// Rename the environment to <name>
        #[bpaf(long, short, argument("name"))]
        name: EnvironmentName,
    },
}

impl Edit {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("edit");

        let detected_environment = self
            .environment
            .detect_concrete_environment(&flox, "edit")?;

        match self.action {
            EditAction::EditManifest { file } => {
                Self::edit_manifest(&flox, detected_environment, file).await?
            },
            EditAction::Rename { name } => {
                if let ConcreteEnvironment::Path(mut environment) = detected_environment {
                    let old_name = environment.name();
                    if name == old_name {
                        bail!("⚠️  environment already named {name}");
                    }
                    environment.rename(name.clone())?;
                    info!("✅  renamed environment {old_name} to {name}");
                } else {
                    // todo: handle remote environments in the future
                    bail!("❌  Cannot rename environments on floxhub");
                }
            },
        }

        Ok(())
    }

    async fn edit_manifest(
        flox: &Flox,
        detected_environment: ConcreteEnvironment,
        file: Option<PathBuf>,
    ) -> Result<()> {
        let active_environment =
            UninitializedEnvironment::from_concrete_environment(&detected_environment)?;
        let mut environment = detected_environment.into_dyn_environment();

        let result = match Self::provided_manifest_contents(file)? {
            // If provided with the contents of a manifest file, either via a path to a file or via
            // contents piped to stdin, use those contents to try building the environment.
            Some(new_manifest) => environment.edit(flox, new_manifest)?,
            // If not provided with new manifest contents, let the user edit the file directly
            // via $EDITOR or $VISUAL (as long as `flox edit` was invoked interactively).
            None => Self::interactive_edit(flox, environment.as_mut()).await?,
        };
        match result {
            EditResult::Unchanged => {
                println!("⚠️  No changes made to environment.");
            },
            EditResult::ReActivateRequired => {
                if activated_environments().is_active(&active_environment) {
                    println!(indoc::indoc! {"
                            Your manifest has changes that cannot be automatically applied to your current environment.

                            Please `exit` the environment and run `flox activate` to see these changes."});
                } else {
                    println!("✅  Environment successfully updated.");
                }
            },
            EditResult::Success => {
                println!("✅  Environment successfully updated.");
            },
        }
        Ok(())
    }

    /// Interactively edit the manifest file
    async fn interactive_edit(
        flox: &Flox,
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
        std::fs::write(&tmp_manifest, environment.manifest_content(flox)?)?;
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

            let result = Dialog {
                message: "Building environment to validate edit...",
                help_message: None,
                typed: Spinner::new(|| environment.edit(flox, new_manifest.clone())),
            }
            .spin();

            match result {
                Err(e) => {
                    error!(
                        "Environment invalid; building resulted in an error: {}",
                        anyhow!(e).chain().join(": ")
                    );
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
    fn provided_manifest_contents(file: Option<PathBuf>) -> Result<Option<String>> {
        if let Some(ref file) = file {
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
    #[bpaf(short, long, hide)]
    force: bool,

    #[allow(dead_code)] // not yet handled in impl
    #[bpaf(short, long, hide)]
    origin: bool,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Delete {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("delete");
        let environment = self
            .environment
            .detect_concrete_environment(&flox, "delete")?;

        let description = environment_description(&environment)?;

        let comfirm = Dialog {
            message: &format!(
                "You are about to delete your environment {description}. Are you sure?"
            ),
            help_message: Some("Use `-f` to force deletion"),
            typed: Confirm {
                default: Some(false),
            },
        };

        if !self.force && Dialog::can_prompt() && !comfirm.prompt().await? {
            bail!("Environment deletion cancelled");
        }

        let result = match environment {
            ConcreteEnvironment::Path(environment) => environment.delete(&flox),
            ConcreteEnvironment::Managed(environment) => environment.delete(&flox),
            ConcreteEnvironment::Remote(environment) => environment.delete(&flox),
        };

        match result {
            Ok(_) => info!("🗑️  environment {description} deleted"),
            Err(err) => Err(err)
                .with_context(|| format!("⚠️  could not delete environment {description}"))?,
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

    /// Trust the a remote environment temporarily for this activation
    #[bpaf(long, short)]
    trust: bool,

    /// Command to run interactively in the context of the environment
    #[bpaf(positional("cmd"), strict, many)]
    run_args: Vec<String>,
}

#[derive(Debug)]
enum ShellType {
    Bash(PathBuf),
    Zsh(PathBuf),
}

impl TryFrom<&Path> for ShellType {
    type Error = anyhow::Error;

    fn try_from(value: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.file_name() {
            Some(name) if name == "bash" => Ok(ShellType::Bash(value.to_owned())),
            Some(name) if name == "zsh" => Ok(ShellType::Zsh(value.to_owned())),
            _ => Err(anyhow!("Unsupported shell {value:?}")),
        }
    }
}

impl Display for ShellType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellType::Bash(_) => write!(f, "bash"),
            ShellType::Zsh(_) => write!(f, "zsh"),
        }
    }
}

impl ShellType {
    /// Detect the current shell from the SHELL environment variable
    ///
    /// TODO:
    /// We want to print an activation script in the format appropriate for the shell that's actually running,
    /// not whatever `SHELL` might be, as `SHELL` might not always be set correctly.
    /// We should detect the type of our parent shell from flox' parent process,
    /// using an approach like [1], rather than blindly trusting `SHELL`.
    ///
    /// [1]: <https://github.com/flox/flox/blob/668a80a40ba19d50f8ca304ff351f4b27a886e21/flox-bash/lib/utils.sh#L1432>
    fn detect() -> Result<Self> {
        let shell = env::var("SHELL").context("SHELL must be set")?;
        let shell = Path::new(&shell);
        let shell = Self::try_from(shell)?;
        Ok(shell)
    }

    fn exe_path(&self) -> &Path {
        match self {
            ShellType::Bash(path) => path,
            ShellType::Zsh(path) => path,
        }
    }
}

impl Activate {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("activate");

        let mut concrete_environment = self.environment.to_concrete_environment(&flox)?;

        // TODO could move this to a pretty print method on the Environment trait?
        let prompt_name = match concrete_environment {
            // Note that the same environment could show up twice without any
            // indication of which comes from which path
            ConcreteEnvironment::Path(ref path) => path.name().to_string(),
            ConcreteEnvironment::Managed(ref managed) => {
                format!("{}/{}", managed.owner(), managed.name())
            },
            ConcreteEnvironment::Remote(ref remote) => {
                format!("{}/{}", remote.owner(), remote.name())
            },
        };

        if let ConcreteEnvironment::Remote(ref env) = concrete_environment {
            if !self.trust {
                ensure_environment_trust(&mut config, &flox, env).await?;
            }
        }

        let now_active =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment)?;

        let environment = concrete_environment.dyn_environment_ref_mut();

        // Don't spin in bashrcs and similar contexts
        let activation_path_result = if !stdout().is_tty() && self.run_args.is_empty() {
            environment.activation_path(&flox)
        } else {
            Dialog {
                message: &format!("Getting ready to use environment {now_active}..."),
                help_message: None,
                typed: Spinner::new(|| environment.activation_path(&flox)),
            }
            .spin()
        };

        let activation_path = match activation_path_result {
            Err(EnvironmentError2::Core(CoreEnvironmentError::LockedManifest(
                LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: 123,
                    ..
                })),
            ))) => {
                let mut message = format!(
                    "This environment is not yet compatible with your system ({system}).",
                    system = flox.system
                );

                if let ConcreteEnvironment::Remote(remote) = &concrete_environment {
                    message.push_str("\n\n");
                    message.push_str(&format!(
                    "Use 'flox pull --add-system {}/{}' to update and verify this environment on your system.",
                    remote.owner(),
                    remote.name()));
                }

                bail!("{message}")
            },
            other => other?,
        };

        // We don't have access to the current PS1 (it's not exported), so we
        // can't modify it. Instead set FLOX_PROMPT_ENVIRONMENTS and let the
        // activation script set PS1 based on that.
        let flox_prompt_environments = env::var(FLOX_PROMPT_ENVIRONMENTS_VAR)
            .map_or(prompt_name.clone(), |prompt_environments| {
                format!("{prompt_name} {prompt_environments}")
            });

        let mut flox_active_environments = activated_environments();

        // Detect if the current environment is already active
        if flox_active_environments.is_active(&now_active) {
            debug!("Environment is already active: environment={now_active}");

            if stdout().is_tty() || !self.run_args.is_empty() {
                // Error if interactive and already active
                bail!("Environment '{now_active}' is already active");
            }

            debug!("Non-interactive shell, ignoring activation (may patch PATH)");
            Self::reactivate_non_interactive()?;
            return Ok(());
        }

        // Add to FLOX_ACTIVE_ENVIRONMENTS so we can detect what environments are active.
        flox_active_environments.set_last_active(now_active.clone());

        // Set FLOX_ENV_DIRS and FLOX_ENV_LIB_DIRS
        let mut flox_env_dirs = IndexSet::from([activation_path.clone()]);
        if let Ok(existing_environments) = env::var(FLOX_ENV_DIRS_VAR) {
            flox_env_dirs.extend(env::split_paths(&existing_environments));
        };
        let (flox_env_dirs_joined, flox_env_lib_dirs_joined) = {
            let flox_env_lib_dirs = flox_env_dirs.iter().map(|p| p.join("lib"));

            let flox_env_dirs = env::join_paths(&flox_env_dirs).context(
                "Cannot activate environment because its path contains an invalid character",
            )?;

            let flox_env_lib_dirs = env::join_paths(flox_env_lib_dirs).context(
                "Cannot activate environment because its path contains an invalid character",
            )?;

            (flox_env_dirs, flox_env_lib_dirs)
        };

        let fixed_up_path_joined = Self::fixup_path(flox_env_dirs).transpose()?;

        let shell = ShellType::detect()?;

        let prompt_color_1 = env::var("FLOX_PROMPT_COLOR_1")
            .unwrap_or(utils::colors::LIGHT_BLUE.to_ansi256().to_string());
        let prompt_color_2 = env::var("FLOX_PROMPT_COLOR_2")
            .unwrap_or(utils::colors::DARK_PEACH.to_ansi256().to_string());

        let exports = HashMap::from([
            (FLOX_ENV_VAR, activation_path.to_string_lossy().to_string()),
            (FLOX_PROMPT_ENVIRONMENTS_VAR, flox_prompt_environments),
            (
                FLOX_ACTIVE_ENVIRONMENTS_VAR,
                flox_active_environments.to_string(),
            ),
            (
                FLOX_ENV_DIRS_VAR,
                flox_env_dirs_joined.to_string_lossy().to_string(),
            ),
            (
                FLOX_ENV_LIB_DIRS_VAR,
                flox_env_lib_dirs_joined.to_string_lossy().to_string(),
            ),
            ("FLOX_PROMPT_COLOR_1", prompt_color_1),
            ("FLOX_PROMPT_COLOR_2", prompt_color_2),
        ]);

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if !stdout().is_tty() && self.run_args.is_empty() {
            Self::activate_non_interactive(
                &shell,
                &exports,
                fixed_up_path_joined,
                &activation_path,
            );

            return Ok(());
        }

        let activate_error =
            Self::activate_interactive(self.run_args, shell, exports, activation_path, now_active);
        // If we get here, exec failed!
        Err(activate_error)
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn activate_interactive(
        run_args: Vec<String>,
        shell: ShellType,
        exports: HashMap<&str, String>,
        activation_path: PathBuf,
        now_active: UninitializedEnvironment,
    ) -> anyhow::Error {
        let mut command = Command::new(shell.exe_path());
        command.envs(exports);

        match shell {
            ShellType::Bash(_) => {
                command
                    .arg("--rcfile")
                    .arg(activation_path.join("activate").join("bash"));
            },
            ShellType::Zsh(_) => {
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
            },
        };

        if !run_args.is_empty() {
            command.arg("-i");
            command.arg("-c");
            command.arg(run_args.join(" "));
        }

        debug!("running activation command: {:?}", command);

        if run_args.is_empty() {
            let message = formatdoc! {"
                ✅  You are now using the environment {now_active}.
                To stop using this environment, type 'exit'"};
            info!("{message}");
        }
        // exec should never return
        command.exec().into()
    }

    /// Patch the PATH to undo the effects of `/usr/libexec/path_helper`
    ///
    /// Darwin has a "path_helper" which indiscriminately reorders the path
    /// to put the system curated path items first in the `PATH`,
    /// which completely breaks the user's ability to manage their `PATH` in subshells,
    /// e.g. when using tmux.
    ///
    /// On macos `/usr/libexec/path_helper` is typically invoked from
    /// default shell rc files, e.g. `/etc/profile` and `/etc/zprofile`.
    ///
    /// This function attempts to undo the effects of `/usr/libexec/path_helper`
    /// by partitioning the `PATH` into two parts:
    /// 1. known paths of activated flox environments
    ///    and nix store paths (e.g. `/nix/store/...`) -- assumed to be `nix develop` paths
    /// 2. everything else
    ///
    /// The `PATH` is then reordered to put the flox environment and nix store paths first.
    /// The order within the two partitions is preserved.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))] // on linux `flox_env_dirs` is not used
    fn fixup_path(flox_env_dirs: IndexSet<PathBuf>) -> Option<Result<OsString>> {
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
        #[cfg(target_os = "macos")]
        {
            if !Path::new("/usr/libexec/path_helper").exists() {
                return None;
            }
            let path_var = env::var("PATH").unwrap_or_default();
            let fixed_up_path = Self::fixup_path_with(path_var, flox_env_dirs);
            let fixed_up_path_joined = env::join_paths(fixed_up_path).context(
                "Cannot activate environment because its path contains an invalid character",
            );
            Some(fixed_up_path_joined)
        }
    }

    /// Patch a given PATH value to undo the effects of `/usr/libexec/path_helper`
    ///
    /// See [Self::fixup_path] for more details.
    fn fixup_path_with(
        path_var: impl AsRef<OsStr>,
        flox_env_dirs: IndexSet<PathBuf>,
    ) -> Vec<PathBuf> {
        let path_iter = env::split_paths(&path_var);

        let (flox_and_nix_path, path) = path_iter.partition::<Vec<_>, _>(|path| {
            let is_flox_path = path
                .parent()
                .map(|path| flox_env_dirs.contains(path))
                .unwrap_or_else(|| flox_env_dirs.contains(path));

            path.starts_with("/nix/store") || is_flox_path
        });

        [flox_and_nix_path, path].into_iter().flatten().collect()
    }

    /// Used when the activated environment is already active
    /// and executed non-interactively -- e.g. from a .bashrc.
    ///
    /// In the general case, this function produces a noop shell function
    ///
    ///     eval "$(flox activate)" -> eval "true"
    ///
    /// On macOS, we need to patch the PATH
    /// to undo the effects of /usr/libexec/path_helper
    ///
    ///     eval "$(flox activate)" -> eval "export PATH=<flox_env_dirs>:$PATH"
    ///
    /// See [Self::fixup_path] for more details.
    fn reactivate_non_interactive() -> Result<(), anyhow::Error> {
        let flox_env_dirs = env::var(FLOX_ENV_DIRS_VAR)
            .ok()
            .as_ref()
            .map(env::split_paths)
            .map(IndexSet::from_iter)
            .unwrap_or_default();
        if let Some(fixed_up_path) = Self::fixup_path(flox_env_dirs) {
            let fixed_up_path_joined = env::join_paths(fixed_up_path).context(
                "Cannot activate environment because its path contains an invalid character",
            )?;
            debug!(
                "Patching PATH to {}",
                fixed_up_path_joined.to_string_lossy()
            );
            println!(
                "export PATH={}",
                shell_escape::escape(fixed_up_path_joined.to_string_lossy())
            );
        } else {
            debug!("No path patching needed");
            println!("true");
        };
        Ok(())
    }

    fn activate_non_interactive(
        shell: &ShellType,
        exports: &HashMap<&str, String>,
        fixed_up_path: Option<Vec<PathBuf>>,
        fixed_up_path_joined: OsString,
        activation_path: &Path,
    ) {
        let exports_rendered = exports
            .iter()
            .map(|(key, value)| (key, shell_escape::escape(Cow::Borrowed(value))))
            .map(|(key, value)| format!("export {key}={value}",))
            .join("\n");

        let path_patch = if fixed_up_path.is_some() {
            formatdoc! {"
                    # Add flox environment to PATH
                    export FLOX_PATH_PATCHED={fixed_up_path_joined}",
        let path_patch = if let Some(fixed_up_path_joined) = fixed_up_path_joined {
                fixed_up_path_joined=shell_escape::escape(fixed_up_path_joined.to_string_lossy()),
        } else {
            "# No path patching needed".to_string()
        };

        let script = formatdoc! {"
                # Common flox environment variables
                {exports_rendered}

                {path_patch}

                # to avoid infinite recursion sourcing bashrc
                export FLOX_SOURCED_FROM_SHELL_RC=1

                source {activation_path}/activate/{shell}

                unset FLOX_SOURCED_FROM_SHELL_RC
            ",
        activation_path=shell_escape::escape(activation_path.to_string_lossy()),
        };

        println!("{script}");
    }
}

#[cfg(test)]
mod activate_tests {
    use super::*;

    const PATH: &str =
        "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/flox/env/bin:/nix/store/some/bin";

    #[test]
    fn test_fixup_path() {
        let flox_env_dirs = IndexSet::from(["/flox/env"].map(PathBuf::from));
        let fixed_up_path = Activate::fixup_path_with(PATH, flox_env_dirs);
        let joined = env::join_paths(fixed_up_path).unwrap();

        assert_eq!(
            joined.to_string_lossy(),
            "/flox/env/bin:/nix/store/some/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "PATH was not reordered correctly"
        );
    }
}

/// Create an environment in the current directory
#[derive(Bpaf, Clone)]
pub struct Init {
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

        let env = PathEnvironment::init(
            PathPointer::new(env_name),
            &dir,
            flox.temp_dir.clone(),
            &flox.system,
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
    #[bpaf(external(list_mode), fallback(ListMode::Extended))]
    list_mode: ListMode,
}

#[derive(Bpaf, Clone)]
pub enum ListMode {
    /// Show the raw contents of the manifest
    #[bpaf(long, short)]
    Config,
    /// Show only names
    #[bpaf(long("name"), short)]
    NameOnly,

    /// Show names, paths, and versions (default)
    #[bpaf(long, short)]
    Extended,

    /// Detailed information such as priority and license
    #[bpaf(long, short)]
    All,
}

impl List {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("list");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "list using")?
            .into_dyn_environment();

        let manifest_contents = env.manifest_content(&flox)?;
        match self.list_mode {
            ListMode::Config => println!("{}", manifest_contents),
            ListMode::NameOnly => self.print_name_only(&flox, &mut *env)?,
            ListMode::Extended => self.print_extended(&flox, &mut *env)?,
            ListMode::All => self.print_detail(&flox, &mut *env)?,
        }

        Ok(())
    }

    /// print package ids only
    fn print_name_only(&self, flox: &Flox, env: &mut dyn Environment) -> Result<()> {
        let lockfile = Self::get_lockfile(flox, env)?;
        lockfile
            .list_packages(&flox.system)
            .into_iter()
            .for_each(|p| println!("{}", p.name));
        Ok(())
    }

    /// print package ids, as well as path and version
    ///
    /// e.g. `pip: python3Packages.pip (20.3.4)`
    ///
    /// This is the default mode
    fn print_extended(&self, flox: &Flox, env: &mut dyn Environment) -> Result<()> {
        let lockfile = Self::get_lockfile(flox, env)?;
        lockfile
            .list_packages(&flox.system)
            .into_iter()
            .for_each(|p| {
                println!(
                    "{id}: {path} ({version})",
                    id = p.name,
                    path = p.rel_path,
                    version = p.info.version
                )
            });
        Ok(())
    }

    /// print package ids, as well as extended detailed information
    fn print_detail(&self, flox: &Flox, env: &mut dyn Environment) -> Result<()> {
        let lockfile = Self::get_lockfile(flox, env)?;

        for InstalledPackage {
            name,
            rel_path,
            info:
                PackageInfo {
                    broken,
                    license,
                    pname,
                    unfree,
                    version,
                    description,
                },
            priority,
        } in lockfile
            .list_packages(&flox.system)
            .into_iter()
            .sorted_by_key(|p| p.priority)
        {
            let message = formatdoc! {"
                {name}: ({pname})
                  Description: {description}
                  Path:     {rel_path}
                  Priority: {priority}
                  Version:  {version}
                  License:  {license}
                  Unfree:   {unfree}
                  Broken:   {broken}
                ",
                description = description.unwrap_or_else(|| "N/A".to_string()),
                license = license.unwrap_or_else(|| "N/A".to_string()),
            };

            println!("{message}");
        }

        Ok(())
    }

    /// Read existing lockfile or resolve to create a new [LockedManifest].
    ///
    /// Does not write the lockfile,
    /// as that would require writing to the environment in case of remote environments)
    fn get_lockfile(flox: &Flox, env: &mut dyn Environment) -> Result<TypedLockedManifest> {
        let lockfile_path = env
            .lockfile_path(flox)
            .context("Could not get lockfile path")?;

        let lockfile = if !lockfile_path.exists() {
            Dialog {
                message: "No lockfile found for environment, building...",
                help_message: None,
                typed: Spinner::new(|| env.lock(flox)),
            }
            .spin()
            .context("Failed to build environment")?
        } else {
            let lockfile_content =
                fs::read_to_string(lockfile_path).context("Could not read lockfile")?;
            serde_json::from_str(&lockfile_content)?
        };

        let lockfile: TypedLockedManifest = lockfile.try_into()?;
        Ok(lockfile)
    }
}

fn environment_description(environment: &ConcreteEnvironment) -> Result<String> {
    Ok(UninitializedEnvironment::from_concrete_environment(environment)?.to_string())
}

/// Install a package into an environment
#[derive(Bpaf, Clone)]
pub struct Install {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Option to specify a package ID
    #[bpaf(external(pkg_with_id_option), many)]
    id: Vec<PkgWithIdOption>,

    #[bpaf(positional("packages"))]
    packages: Vec<String>,
}

#[derive(Debug, Bpaf, Clone)]
#[bpaf(adjacent)]
#[allow(clippy::manual_non_exhaustive)]
pub struct PkgWithIdOption {
    /// Install a package and assign an explicit ID
    #[bpaf(long("id"), short('i'))]
    _option: (),
    /// ID of the package to install
    #[bpaf(positional("id"))]
    pub id: String,
    /// Path to the package to install as shown by `flox search`
    #[bpaf(positional("package"))]
    pub path: String,
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
        let mut packages = self
            .packages
            .iter()
            .map(|p| PackageToInstall::from_str(p))
            .collect::<Result<Vec<_>, _>>()?;
        packages.extend(self.id.iter().map(|p| PackageToInstall {
            id: p.id.clone(),
            path: p.path.clone(),
            version: None,
            input: None,
        }));
        if packages.is_empty() {
            bail!("Must specify at least one package");
        }

        let installation = Dialog {
            message: &format!("Installing packages to environment {description}..."),
            help_message: None,
            typed: Spinner::new(|| environment.install(&packages, &flox)),
        }
        .spin()
        .map_err(|err| Self::handle_error(err, &flox, &*environment, &packages))?;

        if installation.new_manifest.is_some() {
            // Print which new packages were installed
            for pkg in packages.iter() {
                if let Some(false) = installation.already_installed.get(&pkg.id) {
                    info!("✅ '{}' installed to environment {description}", pkg.id);
                } else {
                    info!(
                        "⚠️  Package with id '{}' already installed to environment {description}",
                        pkg.id
                    );
                }
            }
        } else {
            for pkg in packages.iter() {
                info!(
                    "⚠️  Package with id '{}' already installed to environment {description}",
                    pkg.id
                );
            }
        }
        Ok(())
    }

    fn handle_error(
        err: EnvironmentError2,
        flox: &Flox,
        environment: &dyn Environment,
        packages: &[PackageToInstall],
    ) -> anyhow::Error {
        debug!("install error: {:?}", err);

        match err {
            EnvironmentError2::Core(CoreEnvironmentError::LockedManifest(
                LockedManifestError::LockManifest(
                    flox_rust_sdk::models::pkgdb::CallPkgDbError::PkgDbError(pkgdberr),
                ),
            )) if pkgdberr.exit_code == 120 => 'error: {
                let paths = packages.iter().map(|p| p.path.clone()).join(", ");
                let head = format!("❌  could not install {paths}");

                if packages.len() > 1 {
                    break 'error anyhow!(formatdoc! {"
                        {head}
                        One or more of the packages you are trying to install does not exist.
                    "});
                }
                let path = packages[0].path.clone();

                let suggestion = DidYouMean::<InstallSuggestion>::new(flox, environment, &path);
                if !suggestion.has_suggestions() {
                    break 'error anyhow!(head);
                }

                anyhow!(formatdoc! {"
                    {head}
                    {suggestion}
                "})
            },
            _ => err.into(),
        }
    }
}

/// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(positional("packages"), some("Must specify at least one package"))]
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

        let _ = Dialog {
            message: &format!("Uninstalling packages from environment {description}..."),
            help_message: None,
            typed: Spinner::new(|| environment.uninstall(self.packages.clone(), &flox)),
        }
        .spin()?;

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

/// Send environment to floxhub
#[derive(Bpaf, Clone)]
pub struct Push {
    /// Directory to push the environment from (default: current directory)
    #[bpaf(long, short, argument("path"))]
    dir: Option<PathBuf>,

    /// Owner to push push environment to (default: current user)
    #[bpaf(long, short, argument("owner"))]
    owner: Option<EnvironmentOwner>,

    /// forceably overwrite the remote copy of the environment
    #[bpaf(long, short)]
    force: bool,
}

impl Push {
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("push");

        if flox.floxhub_token.is_none() {
            if !Dialog::can_prompt() {
                let message = formatdoc! {"
                    You are not logged in to floxhub.

                    Can not automatically login to floxhub in non-interactive context.

                    To login you can either
                    * login to floxhub with 'flox auth login',
                    * set the 'floxhub_token' field to '<your token>' in your config
                    * set the '$FLOX_FLOXHUB_TOKEN=<your_token>' environment variable."
                };
                bail!(message);
            }

            info!("You are not logged in to floxhub. Logging in...");

            auth::login_flox(&mut flox).await?;
        }

        let dir = self.dir.unwrap_or_else(|| std::env::current_dir().unwrap());

        match EnvironmentPointer::open(&dir)? {
            EnvironmentPointer::Managed(managed_pointer) => {
                let message = Self::push_existing_message(&managed_pointer, self.force);

                Self::push_managed_env(&flox, managed_pointer, dir, self.force)?;

                info!("{message}");
            },

            EnvironmentPointer::Path(path_pointer) => {
                let owner = if let Some(owner) = self.owner {
                    owner
                } else {
                    EnvironmentOwner::from_str(
                        &flox
                            .floxhub_token
                            .as_ref()
                            .context("Need to be loggedin")?
                            .handle()?,
                    )?
                };
                let env = Self::push_make_managed(&flox, path_pointer, &dir, owner, self.force)?;

                info!("{}", Self::push_new_message(env.pointer(), self.force));
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
        let mut env = ManagedEnvironment::open(flox, managed_pointer.clone(), dir.join(DOT_FLOX))
            .context("Could not open environment")?;
        env.push(flox, force)
            .map_err(|err| Self::convert_error(err, managed_pointer, false))?;

        Ok(())
    }

    /// pushes a path environment in a directory to floxhub and makes it a managed environment
    fn push_make_managed(
        flox: &Flox,
        path_pointer: PathPointer,
        dir: &Path,
        owner: EnvironmentOwner,
        force: bool,
    ) -> Result<ManagedEnvironment> {
        let dot_flox_path = dir.join(DOT_FLOX);
        let path_environment =
            path_environment::PathEnvironment::open(path_pointer, dot_flox_path, &flox.temp_dir)?;

        let pointer = ManagedPointer::new(owner.clone(), path_environment.name(), &flox.floxhub);

        let env = ManagedEnvironment::push_new(flox, path_environment, owner, force)
            .map_err(|err| Self::convert_error(err, pointer, true))?;

        Ok(env)
    }

    fn convert_error(
        err: ManagedEnvironmentError,
        pointer: ManagedPointer,
        create_remote: bool,
    ) -> anyhow::Error {
        let owner = &pointer.owner;
        let name = &pointer.name;

        fn error_chain(mut e: &dyn Error) -> String {
            let mut msg = e.to_string();
            while let Some(source) = e.source() {
                e = source;
                msg.push_str(&format!(": {}", e));
            }
            msg
        }

        let message = match err {
            ManagedEnvironmentError::AccessDenied => formatdoc! {"
                ❌  You do not have permission to write to {owner}/{name}
            "}.into(),
            ManagedEnvironmentError::Diverged if create_remote => formatdoc! {"
                ❌  An environment named {owner}/{name} already exists!

                To rename your environment: 'flox edit --name <new name>'
                To pull and manually re-apply your changes: 'flox delete && flox pull -r {owner}/{name}'
            "}.into(),
            ManagedEnvironmentError::Build(ref err) => formatdoc! {"
                {err}

                ❌  Unable to push environment with build errors.

                Use 'flox edit' to resolve errors, test with 'flox activate', and 'flox push' again.",
                err = error_chain(err)
            }.into(),
            _ => None
        };

        // todo: add message to error using `context` when we work more on polishing errors
        if let Some(message) = message {
            debug!("converted error to message: {err:?} -> {message}");
            anyhow::Error::msg(message)
        } else {
            err.into()
        }
    }

    /// construct a message for an updated environment
    ///
    /// todo: add floxhub base url when it's available
    fn push_existing_message(env: &ManagedPointer, force: bool) -> String {
        let owner = &env.owner;
        let name = &env.name;

        let suffix = if force { " (forced)" } else { "" };

        formatdoc! {"
            ✅  Updates to {name} successfully pushed to floxhub{suffix}

            Use 'flox pull {owner}/{name}' to get this environment in any other location.
        "}
    }

    /// construct a message for a newly created environment
    ///
    /// todo: add floxhub base url when it's available
    fn push_new_message(env: &ManagedPointer, force: bool) -> String {
        let owner = &env.owner;
        let name = &env.name;

        let suffix = if force { " (forced)" } else { "" };

        formatdoc! {"
            ✅  {name} successfully pushed to floxhub{suffix}

            Use 'flox pull {owner}/{name}' to get this environment in any other location.
        "}
    }
}

#[derive(Debug, Clone, Bpaf)]
enum PullSelect {
    New {
        /// Directory to create the environment in (default: current directory)
        #[bpaf(long, short, argument("path"))]
        dir: Option<PathBuf>,
        /// ID of the environment to pull
        #[bpaf(long, short, argument("owner/name"))]
        remote: EnvironmentRef,
    },
    NewAbbreviated {
        /// Directory to create the environment in (default: current directory)
        #[bpaf(long, short, argument("path"))]
        dir: Option<PathBuf>,
        /// ID of the environment to pull
        #[bpaf(positional("owner/name"))]
        remote: EnvironmentRef,
    },
    Existing {
        /// Directory containing a managed environment to pull
        #[bpaf(long, short, argument("path"))]
        dir: Option<PathBuf>,
        /// Forceably overwrite the local copy of the environment
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

/// Pull environment from floxhub
#[derive(Bpaf, Clone)]
pub struct Pull {
    /// Forceably add current systems to the environment, even if incompatible
    #[bpaf(long("add-system"), short)]
    add_system: bool,

    #[bpaf(external(pull_select), fallback(Default::default()))]
    pull_select: PullSelect,
}

impl Pull {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("pull");

        match self.pull_select {
            PullSelect::New { dir, remote } | PullSelect::NewAbbreviated { dir, remote } => {
                let (start, complete) =
                    Self::pull_new_messages(dir.as_deref(), &remote, flox.floxhub.base_url());

                let dir = dir.unwrap_or_else(|| std::env::current_dir().unwrap());

                debug!("Resolved user intent: pull {remote:?} into {dir:?}");

                Self::pull_new_environment(
                    &flox,
                    dir.join(DOT_FLOX),
                    remote,
                    self.add_system,
                    &start,
                )?;

                info!("{complete}");
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

                let (start, complete) = Self::pull_existing_messages(&pointer, force);
                info!("{start}");

                Dialog {
                    message: &start,
                    help_message: None,
                    typed: Spinner::new(|| {
                        Self::pull_existing_environment(&flox, dir.join(DOT_FLOX), pointer, force)
                    }),
                }
                .spin()?;

                info!("{complete}");
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
        env.build(flox).context("Could not build environment")?;

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
        add_systems: bool,
        message: &str,
    ) -> Result<()> {
        if dot_flox_path.exists() {
            bail!("Cannot pull a new environment into an existing one")
        }
        let pointer = ManagedPointer::new(
            env_ref.owner().clone(),
            env_ref.name().clone(),
            &flox.floxhub,
        );

        let pointer_content =
            serde_json::to_string_pretty(&pointer).context("Could not serialize pointer")?;

        let dot_flox_parent = dot_flox_path
            .parent()
            .context("Could not get .flox/ parent")?;
        fs::create_dir_all(dot_flox_parent).context("Could not create .flox/ parent directory")?;

        // Pulls the environment into a temp directory which is later renamed to `.flox`.
        // We do this to avoid populating the floxmeta branch `owner/name.encode(path to .flox)`
        // in case building fails or the user aborts the fixup.
        // The branch will be adjusted when the environment is opened the next time
        // by the `ensure_branch` routine.
        let temp_dot_flox_dir = tempfile::TempDir::with_prefix_in(DOT_FLOX, dot_flox_parent)
            .context("Could not create temporary directory for cloning environment")?;

        let pointer_path = temp_dot_flox_dir.path().join(ENVIRONMENT_POINTER_FILENAME);
        fs::write(pointer_path, pointer_content).context("Could not write pointer")?;

        let mut env = {
            let result = Dialog {
                message,
                help_message: None,
                typed: Spinner::new(|| ManagedEnvironment::open(flox, pointer, &temp_dot_flox_dir)),
            }
            .spin()
            .map_err(Self::convert_error);

            match result {
                Err(err) => {
                    fs::remove_dir_all(&temp_dot_flox_dir)
                        .context("Could not clean up .flox/ directory")?;
                    Err(err)?
                },
                Ok(env) => env,
            }
        };

        let result = Dialog {
            message,
            help_message: None,
            typed: Spinner::new(|| env.build(flox)),
        }
        .spin();

        match result {
            Ok(_) => {
                fs::rename(temp_dot_flox_dir, dot_flox_path)
                    .context("Could not move .flox/ directory")?;
            },
            Err(EnvironmentError2::Core(CoreEnvironmentError::LockedManifest(
                LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: 123,
                    ..
                })),
            ))) => {
                let hint = "Use 'flox pull --add-system' to add your system to the manifest.";

                // will return OK if the user chose to abort the pull
                let add_systems = add_systems
                    || match Self::query_add_system(&flox.system)? {
                        Some(false) => {
                            // prompt available, user chose to abort
                            info!("{hint}");
                            bail!("Did not pull the environment.");
                        },
                        Some(true) => true, // prompt available, user chose to add system
                        None => false,      // no prompt available
                    };

                if !add_systems {
                    bail!("{}", formatdoc! {"
                        This environment is not yet compatible with your system ({system}).

                        {hint}"
                    , system = flox.system});
                }

                let doc = Self::amend_current_system(&env, flox)?;
                if let Err(broken_error) = env.edit_unsafe(flox, doc.to_string())? {
                    warn!("{}", formatdoc! {"
                        {err:#}

                        Could not build modified environment, build errors need to be resolved manually.",
                        err = anyhow!(broken_error)
                    });
                };

                fs::rename(temp_dot_flox_dir, dot_flox_path)
                    .context("Could not move .flox/ directory")?;
            },
            Err(e) => {
                fs::remove_dir_all(&temp_dot_flox_dir)
                    .context("Could not clean up .flox/ directory")?;
                Err(e)?
            },
        }

        Ok(())
    }

    fn convert_error(err: ManagedEnvironmentError) -> anyhow::Error {
        if let ManagedEnvironmentError::OpenFloxmeta(FloxmetaV2Error::LoggedOut) = err {
            anyhow!(indoc! {"
                Could not pull environment: not logged in to floxhub.

                Please login to floxhub with `flox auth login`
                "})
        } else {
            anyhow!(err)
        }
    }

    /// construct a message for pulling a new environment
    fn pull_new_messages(
        dir: Option<&Path>,
        env_ref: &EnvironmentRef,
        floxhub_host: &Url,
    ) -> (String, String) {
        let mut start_message =
            format!("⬇️  Remote: pulling and building {env_ref} from {floxhub_host}");
        if let Some(dir) = dir {
            start_message += &format!(" into {dir}", dir = dir.display());
        } else {
            start_message += " into the current directory";
        };

        let complete_message = formatdoc! {"
            ✨  Pulled {env_ref} from {floxhub_host}

            You can activate this environment with 'flox activate'
        "};

        (start_message, complete_message)
    }

    /// construct a message for pulling an existing environment
    ///
    /// todo: add floxhub base url when it's available
    fn pull_existing_messages(pointer: &ManagedPointer, force: bool) -> (String, String) {
        let owner = &pointer.owner;
        let name = &pointer.name;
        let floxhub_host = &pointer.floxhub_url;

        let start_message =
            format!("⬇️  Remote: pulling and building {owner}/{name} from {floxhub_host}",);

        let suffix: &str = if force { " (forced)" } else { "" };

        let complete_message = formatdoc! {"
            ✨  Pulled {owner}/{name} from {floxhub_host}{suffix}

            You can activate this environment with 'flox activate'
        "};

        (start_message, complete_message)
    }

    /// if possible, prompt the user to automatically add their system to the manifest
    ///
    /// returns [Ok(None)]` if the user can't be prompted
    /// returns `[Ok(bool)]` depending on the users choice
    /// returns `[Err]` if the prompt failed or was cancelled
    fn query_add_system(system: &str) -> Result<Option<bool>> {
        if !Dialog::can_prompt() {
            return Ok(None);
        }

        let message = format!(
        "The environment you are trying to pull is not yet compatible with your system ({system})."
    );
        let help = "Use 'flox pull --add-system' to automatically add your system to the list of compatible systems";

        let reject_choice = "Don't pull this environment.";
        let confirm_choice = format!(
            "Pull this environment anyway and add '{system}' to the supported systems list."
        );

        let dialog = Dialog {
            message: &message,
            help_message: Some(help),
            typed: Select {
                options: [reject_choice, &confirm_choice].to_vec(),
            },
        };

        let (choice, _) = dialog.raw_prompt()?;

        Ok(Some(choice == 1))
    }

    /// add the current system to the manifest of the given environment
    fn amend_current_system(
        env: &ManagedEnvironment,
        flox: &Flox,
    ) -> Result<Document, anyhow::Error> {
        manifest::add_system(&env.manifest_content(flox)?, &flox.system)
            .context("Could not add system to manifest")
    }
}

/// rollback to the previous generation of an environment
#[derive(Bpaf, Clone)]
pub struct Rollback {
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

#[derive(Debug, Bpaf, Clone)]
pub enum EnvironmentOrGlobalSelect {
    Environment(#[bpaf(external(environment_select))] EnvironmentSelect),
    /// Update inputs used by 'search' and 'show' outside of an environment
    #[bpaf(long("global"))]
    Global,
}

impl Default for EnvironmentOrGlobalSelect {
    fn default() -> Self {
        EnvironmentOrGlobalSelect::Environment(Default::default())
    }
}

/// Update an environment's inputs
#[derive(Bpaf, Clone)]
pub struct Update {
    #[bpaf(external(environment_or_global_select), fallback(Default::default()))]
    environment_or_global: EnvironmentOrGlobalSelect,

    #[bpaf(positional("inputs"))]
    inputs: Vec<String>,
}
impl Update {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("update");

        let (old_lockfile, new_lockfile, global, description) = match self.environment_or_global {
            EnvironmentOrGlobalSelect::Environment(ref environment_select) => {
                let concrete_environment =
                    environment_select.detect_concrete_environment(&flox, "update")?;

                let description = Some(environment_description(&concrete_environment)?);
                let (old_manifest, new_manifest) =
                    self.update_manifest(flox, concrete_environment)?;
                (
                    old_manifest
                        .map(TypedLockedManifest::try_from)
                        .transpose()?,
                    TypedLockedManifest::try_from(new_manifest)?,
                    false,
                    description,
                )
            },
            EnvironmentOrGlobalSelect::Global => {
                let (old_manifest, new_manifest) =
                    LockedManifest::update_global_manifest(&flox, self.inputs)?;
                (
                    old_manifest
                        .map(TypedLockedManifest::try_from)
                        .transpose()?,
                    TypedLockedManifest::try_from(new_manifest)?,
                    true,
                    None,
                )
            },
        };

        if let Some(ref old_lockfile) = old_lockfile {
            if new_lockfile.registry().inputs == old_lockfile.registry().inputs {
                if global {
                    info!("ℹ️  All global inputs are up-to-date.");
                } else {
                    info!(
                        "ℹ️  All inputs are up-to-date in environment {}.",
                        description.as_ref().unwrap()
                    );
                }

                return Ok(());
            }
        }

        let mut inputs_to_scrape: Vec<&Input> = vec![];

        for (input_name, new_input) in &new_lockfile.registry().inputs {
            let old_input = old_lockfile
                .as_ref()
                .and_then(|old| old.registry().inputs.get(input_name));
            match old_input {
                // unchanged input
                Some(old_input) if old_input == new_input => continue, // dont need to scrape
                // updated input
                Some(_) if global => info!("⬆️  Updated global input '{}'.", input_name),
                Some(_) => info!(
                    "⬆️  Updated input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap()
                ),
                // new input
                None if global => info!("🔒️  Locked global input '{}'.", input_name),
                None => info!(
                    "🔒️  Locked input '{}' in environment {}.",
                    input_name,
                    description.as_ref().unwrap(),
                ),
            }
            inputs_to_scrape.push(new_input);
        }

        if let Some(old_lockfile) = old_lockfile {
            for input_name in old_lockfile.registry().inputs.keys() {
                if !new_lockfile.registry().inputs.contains_key(input_name) {
                    if global {
                        info!(
                            "🗑️  Removed unused input '{}' from global lockfile.",
                            input_name
                        );
                    } else {
                        info!(
                            "🗑️  Removed unused input '{}' from lockfile for environment {}.",
                            input_name,
                            description.as_ref().unwrap()
                        );
                    }
                }
            }
        }

        if inputs_to_scrape.is_empty() {
            return Ok(());
        }

        // TODO: make this async when scraping multiple inputs
        let results: Vec<Result<()>> = Dialog {
            message: "Generating databases for updated inputs...",
            help_message: (inputs_to_scrape.len() > 1).then_some("This may take a while."),
            typed: Spinner::new(|| {
                inputs_to_scrape
                    .iter() // TODO: rayon::par_iter
                    .map(|input| Self::scrape_input(&input.from))
                    .collect()
            }),
        }
        .spin();

        for result in results {
            result?;
        }

        Ok(())
    }

    fn update_manifest(
        &self,
        flox: Flox,
        concrete_environment: ConcreteEnvironment,
    ) -> Result<UpdateResult> {
        let mut environment = concrete_environment.into_dyn_environment();

        environment
            .update(&flox, self.inputs.clone())
            .context("updating environment failed")
    }

    fn scrape_input(input: &FlakeRef) -> Result<()> {
        let mut pkgdb_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_cmd
            .args(["scrape"])
            .arg(serde_json::to_string(&input)?)
            // TODO: this works for nixpkgs, but it won't work for anything else
            .arg("legacyPackages");

        debug!("scraping input: {pkgdb_cmd:?}");
        call_pkgdb(pkgdb_cmd)?;
        Ok(())
    }
}

#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// ID of a package or group name to upgrade
    #[bpaf(positional("package or group"))]
    groups_or_iids: Vec<String>,
}
impl Upgrade {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");

        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "upgrade")?;

        let description = environment_description(&concrete_environment)?;

        let mut environment = concrete_environment.into_dyn_environment();

        let upgraded = environment
            .upgrade(&flox, &self.groups_or_iids)
            .context("upgrading environment failed")?
            .0;

        if upgraded.is_empty() {
            if self.groups_or_iids.is_empty() {
                info!("ℹ️  No packages need to be upgraded in environment {description}.");
            } else {
                info!(
                    "ℹ️  The specified packages do not need to be upgraded in environment {description}."
                );
            }
        } else {
            for package in upgraded {
                info!("⬆️  Upgraded '{package}' in environment {description}.");
            }
        }

        Ok(())
    }
}

#[derive(Bpaf, Clone, Debug)]
pub struct Containerize {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Path to write the container to (pass '-' to write to stdout)
    #[bpaf(short, long, argument("path"))]
    output: Option<PathBuf>,
}
impl Containerize {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("containerize");

        let mut env = self
            .environment
            .detect_concrete_environment(&flox, "upgrade")?
            .into_dyn_environment();

        let output_path = match self.output {
            Some(output) => output,
            None => std::env::current_dir()
                .context("Could not get current directory")?
                .join(format!("{}-container.tar.gz", env.name())),
        };

        let (output, output_name): (Box<dyn Write>, String) = if output_path == Path::new("-") {
            debug!("output=stdout");

            (Box::new(std::io::stdout()), "stdout".to_string())
        } else {
            debug!("output={}", output_path.display());

            let file = fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&output_path)
                .context("Could not open output file")?;

            (Box::new(file), output_path.display().to_string())
        };

        let builder = Dialog {
            message: &format!("Building container for environment {}...", env.name()),
            help_message: None,
            typed: Spinner::new(|| env.build_container(&flox)),
        }
        .spin()
        .context("could not create container builder")?;

        info!("Writing container to '{output_name}'");

        builder
            .stream_container(output)
            .context("could not write container to output")?;

        info!("✨  Container written to '{output_name}'");
        Ok(())
    }
}
