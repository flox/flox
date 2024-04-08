use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
#[cfg(target_os = "macos")]
use std::ffi::OsStr;
use std::ffi::OsString;
use std::io::stdout;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use bpaf::Bpaf;
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    Environment,
    EnvironmentError,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
    FLOX_ENV_CACHE_VAR,
    FLOX_ENV_DIRS_VAR,
    FLOX_ENV_LIB_DIRS_VAR,
    FLOX_ENV_PROJECT_VAR,
    FLOX_ENV_VAR,
    FLOX_PATH_PATCHED_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR,
};
use flox_rust_sdk::models::lockfile::LockedManifestError;
use flox_rust_sdk::models::pkgdb::{error_codes, CallPkgDbError, PkgDbError};
use indexmap::IndexSet;
use indoc::formatdoc;
use itertools::Itertools;
use log::{debug, warn};

use super::{
    activated_environments,
    environment_select,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::commands::{ensure_environment_trust, ConcreteEnvironment, EnvironmentSelectError};
use crate::config::Config;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::openers::Shell;
use crate::utils::{default_nix_env_vars, message};
use crate::{subcommand_metric, utils};

/// When called with no arguments 'flox activate' will look for a '.flox' directory
/// in the current directory. Calling 'flox activate' in your home directory will
/// activate a default environment. Environments in other directories and remote
/// environments are activated with the '-d' and '-r' flags respectively.
#[derive(Bpaf, Clone)]
pub struct Activate {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Trust a remote environment temporarily for this activation
    #[bpaf(long, short)]
    trust: bool,

    /// Print an activation script to stdout instead of spawning a subshell
    #[bpaf(long("print-script"), short, hide)]
    print_script: bool,

    /// Command to run interactively in the context of the environment
    #[bpaf(positional("cmd"), strict, many)]
    run_args: Vec<String>,
}

impl Activate {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        subcommand_metric!("activate");
        let mut concrete_environment = match self.environment.to_concrete_environment(&flox) {
            Ok(concrete_environment) => concrete_environment,
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
            {e}

            Create an environment with 'flox init'"
                })
            },
            Err(e) => Err(e)?,
        };

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

        let in_place = self.print_script || (!stdout().is_tty() && self.run_args.is_empty());
        // Don't spin in bashrcs and similar contexts
        let activation_path_result = if in_place {
            environment.activation_path(&flox)
        } else {
            Dialog {
                message: &format!(
                    "Preparing environment {}...",
                    now_active.message_description()?
                ),
                help_message: None,
                typed: Spinner::new(|| environment.activation_path(&flox)),
            }
            .spin()
        };

        let activation_path = match activation_path_result {
            Err(EnvironmentError::Core(CoreEnvironmentError::LockedManifest(
                LockedManifestError::BuildEnv(CallPkgDbError::PkgDbError(PkgDbError {
                    exit_code: error_codes::LOCKFILE_INCOMPATIBLE_SYSTEM,
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
                    "Use 'flox pull --force {}/{}' to update and verify this environment on your system.",
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

        // install prefixes of all active environments
        let flox_env_install_prefixes = IndexSet::from_iter(env::split_paths(
            &env::var(FLOX_ENV_DIRS_VAR).unwrap_or_default(),
        ));

        // on macos: patch the existing PATH
        // If this is [Some] the path will be restored from `$FLOX_PATH_PATCHED`
        // As part of running $FLOX_ENV/etc/profile.d/0100_common-paths.sh during activation.
        //
        // NOTE: this does _not_ include any additions to the PATH
        // due to the newly activated environment.
        // Amending the path is strictly implemented by the activation scripts!
        let fixed_up_original_path_joined =
            Self::fixup_path(&flox_env_install_prefixes).transpose()?;

        // Detect if the current environment is already active
        if flox_active_environments.is_active(&now_active) {
            if !in_place {
                // Error if interactive and already active
                bail!(
                    "Environment '{}' is already active.",
                    now_active.bare_description()?
                );
            }
            debug!("Environment is already active: environment={}. Ignoring activation (may patch PATH)", now_active.bare_description()?);
            Self::reactivate_in_place(fixed_up_original_path_joined)?;
            return Ok(());
        }

        // Add to _FLOX_ACTIVE_ENVIRONMENTS so we can detect what environments are active.
        flox_active_environments.set_last_active(now_active.clone());

        // Prepend the new environment to the list of active environments
        let flox_env_install_prefixes = {
            let mut set = IndexSet::from([activation_path.clone()]);
            set.extend(flox_env_install_prefixes);
            set
        };

        // Set FLOX_ENV_DIRS and FLOX_ENV_LIB_DIRS

        let (flox_env_dirs_joined, flox_env_lib_dirs_joined) = {
            let flox_env_lib_dirs = flox_env_install_prefixes.iter().map(|p| p.join("lib"));

            let flox_env_dirs = env::join_paths(&flox_env_install_prefixes).context(
                "Cannot activate environment because its path contains an invalid character",
            )?;

            let flox_env_lib_dirs = env::join_paths(flox_env_lib_dirs).context(
                "Cannot activate environment because its path contains an invalid character",
            )?;

            (flox_env_dirs, flox_env_lib_dirs)
        };

        let prompt_color_1 = env::var("FLOX_PROMPT_COLOR_1")
            .unwrap_or(utils::colors::INDIGO_400.to_ansi256().to_string());
        let prompt_color_2 = env::var("FLOX_PROMPT_COLOR_2")
            .unwrap_or(utils::colors::INDIGO_300.to_ansi256().to_string());

        let mut exports = HashMap::from([
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
            (
                FLOX_ENV_CACHE_VAR,
                environment.cache_path()?.to_string_lossy().to_string(),
            ),
            (
                FLOX_ENV_PROJECT_VAR,
                environment.project_path()?.to_string_lossy().to_string(),
            ),
            ("FLOX_PROMPT_COLOR_1", prompt_color_1),
            ("FLOX_PROMPT_COLOR_2", prompt_color_2),
        ]);

        exports.extend(default_nix_env_vars());

        if let Some(fixed_up_original_path_joined) = fixed_up_original_path_joined {
            exports.insert(
                FLOX_PATH_PATCHED_VAR,
                fixed_up_original_path_joined.to_string_lossy().to_string(),
            );
        }

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if in_place {
            let shell = Self::detect_shell_for_in_place()?;
            Self::activate_in_place(&shell, &exports, &activation_path);

            return Ok(());
        }

        let shell = Self::detect_shell_for_subshell()?;
        // These functions will only return if exec fails
        if !self.run_args.is_empty() {
            Self::activate_command(self.run_args, shell, exports, activation_path)
        } else {
            Self::activate_interactive(shell, exports, activation_path, now_active)
        }
    }

    /// Used for `flox activate -- run_args`
    fn activate_command(
        run_args: Vec<String>,
        shell: Shell,
        exports: HashMap<&str, String>,
        activation_path: PathBuf,
    ) -> Result<()> {
        let mut command = Command::new(shell.exe_path());

        command.envs(exports);

        // TODO: the activation script sets prompt, which isn't necessary
        let script = formatdoc! {r#"
                # to avoid infinite recursion sourcing bashrc
                export FLOX_SOURCED_FROM_SHELL_RC=1

                source {activation_path}/activate/{shell}

                unset FLOX_SOURCED_FROM_SHELL_RC

                {quoted_args}
        "#,
            activation_path=shell_escape::escape(activation_path.to_string_lossy()),
            quoted_args = Self::quote_run_args(&run_args)
        };

        command.arg("-c");
        command.arg(script);

        debug!("running activation command: {:?}", command);

        // exec should never return
        Err(command.exec().into())
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn activate_interactive(
        shell: Shell,
        exports: HashMap<&str, String>,
        activation_path: PathBuf,
        now_active: UninitializedEnvironment,
    ) -> Result<()> {
        let mut command = Command::new(shell.exe_path());
        command.envs(exports);

        match shell {
            Shell::Bash(_) => {
                command
                    .arg("--rcfile")
                    .arg(activation_path.join("activate").join("bash"));
            },
            Shell::Zsh(_) => {
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

        debug!("running activation command: {:?}", command);

        let message = formatdoc! {"
                You are now using the environment {}.
                To stop using this environment, type 'exit'\n", now_active.message_description()?};
        message::updated(message);

        // exec should never return
        Err(command.exec().into())
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
    /// Note: since the "path_helper" i only invoked by login shells,
    /// we only need to setup the PATH patching for `flox activate` in shell rc files.
    ///
    /// ## Example
    ///
    /// > User has `eval "$(flox activate)"` in their `.zshrc`.
    ///
    ///  Without the path patching, the following happens:
    ///
    /// 1. Open a new terminal (login shell)
    ///     -> `path_helper` runs (`PATH=<default envs>`)
    ///     -> `flox activate` runs (`PATH=<flox_env>:<default envs>`)
    /// 2. Open a new tmux session (login shell by default)
    ///     -> `path_helper` runs (`PATH=<default envs>:<flox_env>`)
    ///     -> `flox activate` runs
    ///     -> ⚡️ environment already active, activate skipped
    ///        without path patching: `PATH:<default envs>:<flox_env>` ❌
    ///        with path patching: `PATH:<flox_env>:<default envs>`    ✅ flox env is not shadowed
    ///
    /// ## Implementation
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
    fn fixup_path(flox_env_dirs: &IndexSet<PathBuf>) -> Option<Result<OsString>> {
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
    #[cfg(target_os = "macos")]
    fn fixup_path_with(
        path_var: impl AsRef<OsStr>,
        flox_env_dirs: &IndexSet<PathBuf>,
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
    fn reactivate_in_place(fixed_up_path_joined: Option<OsString>) -> Result<(), anyhow::Error> {
        if let Some(fixed_up_path_joined) = fixed_up_path_joined {
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
        };
        Ok(())
    }

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(shell: &Shell, exports: &HashMap<&str, String>, activation_path: &Path) {
        let exports_rendered = exports
            .iter()
            .map(|(key, value)| (key, shell_escape::escape(Cow::Borrowed(value))))
            .map(|(key, value)| format!("export {key}={value}",))
            .join("\n");

        let script = formatdoc! {"
                # Common flox environment variables
                {exports_rendered}

                # to avoid infinite recursion sourcing bashrc
                export FLOX_SOURCED_FROM_SHELL_RC=1

                source {activation_path}/activate/{shell}

                unset FLOX_SOURCED_FROM_SHELL_RC
            ",
        activation_path=shell_escape::escape(activation_path.to_string_lossy()),
        };

        println!("{script}");
    }

    /// Quote run args so that words don't get split,
    /// but don't escape all characters.
    ///
    /// To do this we escape `"`,
    /// but we don't escape anything else.
    /// We want `$` for example to be expanded by the shell.
    fn quote_run_args(run_args: &[String]) -> String {
        run_args
            .iter()
            .map(|arg| format!(r#""{}""#, arg.replace('"', r#"\""#)))
            .join(" ")
    }

    /// Detect the shell to use for activation
    ///
    /// Used to determine shell for
    /// `flox activate` and `flox activate -- CMD`
    fn detect_shell_for_subshell() -> Result<Shell> {
        Shell::detect_from_env("FLOX_SHELL").or_else(|_| Shell::detect_from_env("SHELL"))
    }

    /// Detect the shell to use for in-place activation
    ///
    /// Used to determine shell for `eval "$(flox activate)"` / `flox activate --print-script`
    fn detect_shell_for_in_place() -> Result<Shell> {
        Self::detect_shell_for_in_place_with(Shell::detect_from_parent_process)
    }

    /// Utility method for testing implementing the logic of shell detection
    /// for in-place activation, generically over a parent shell detection function.
    fn detect_shell_for_in_place_with(
        parent_shell_fn: impl Fn() -> Result<Shell>,
    ) -> Result<Shell> {
        Shell::detect_from_env("FLOX_SHELL")
            .or_else(|_| parent_shell_fn())
            .or_else(|err| {
                warn!("Failed to detect shell from environment: {err}");
                Shell::detect_from_env("SHELL")
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    const PATH: &str =
        "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/flox/env/bin:/nix/store/some/bin";

    #[test]
    #[cfg(target_os = "macos")]
    fn test_fixup_path() {
        let flox_env_dirs = IndexSet::from(["/flox/env"].map(PathBuf::from));
        let fixed_up_path = Activate::fixup_path_with(PATH, &flox_env_dirs);
        let joined = env::join_paths(fixed_up_path).unwrap();

        assert_eq!(
            joined.to_string_lossy(),
            "/flox/env/bin:/nix/store/some/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin",
            "PATH was not reordered correctly"
        );
    }

    const SHELL_SET: (&'_ str, Option<&'_ str>) = ("SHELL", Some("/shell/bash"));
    const FLOX_SHELL_SET: (&'_ str, Option<&'_ str>) = ("FLOX_SHELL", Some("/flox_shell/bash"));
    const SHELL_UNSET: (&'_ str, Option<&'_ str>) = ("SHELL", None);
    const FLOX_SHELL_UNSET: (&'_ str, Option<&'_ str>) = ("FLOX_SHELL", None);
    const PARENT_DETECTED: &dyn Fn() -> Result<Shell> = &|| Ok(Shell::Bash("/parent/bash".into()));
    const PARENT_UNDETECTED: &dyn Fn() -> Result<Shell> =
        &|| Err(anyhow::anyhow!("parent shell detection failed"));

    #[test]
    fn test_detect_shell_for_subshell() {
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_SET], || {
            let shell = Activate::detect_shell_for_subshell().unwrap();
            assert_eq!(shell, Shell::Bash("/shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = Activate::detect_shell_for_subshell().unwrap();
            assert_eq!(shell, Shell::Bash("/flox_shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = Activate::detect_shell_for_subshell();
            assert!(shell.is_err());
        });
    }

    #[test]
    fn test_detect_shell_for_in_place() {
        // $SHELL is used as a fallback only if parent detection fails
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_SET], || {
            let shell = Activate::detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, Shell::Bash("/parent/bash".into()));

            // fall back to $SHELL if parent detection fails
            let shell = Activate::detect_shell_for_in_place_with(PARENT_UNDETECTED).unwrap();
            assert_eq!(shell, Shell::Bash("/shell/bash".into()));
        });

        // $FLOX_SHELL takes precedence over $SHELL and detected parent shell
        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = Activate::detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, Shell::Bash("/flox_shell/bash".into()));

            let shell = Activate::detect_shell_for_in_place_with(PARENT_UNDETECTED).unwrap();
            assert_eq!(shell, Shell::Bash("/flox_shell/bash".into()));
        });

        // if both $FLOX_SHELL and $SHELL are unset, we should fail iff parent detection fails
        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = Activate::detect_shell_for_in_place_with(PARENT_DETECTED).unwrap();
            assert_eq!(shell, Shell::Bash("/parent/bash".into()));

            let shell = Activate::detect_shell_for_in_place_with(PARENT_UNDETECTED);
            assert!(shell.is_err());
        });
    }

    #[test]
    fn test_quote_run_args() {
        assert_eq!(
            Activate::quote_run_args(&["a b".to_string(), '"'.to_string()]),
            r#""a b" "\"""#
        )
    }
}
