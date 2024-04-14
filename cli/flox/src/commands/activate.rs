use std::borrow::Cow;
use std::collections::HashMap;
use std::env;
use std::io::stdout;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Result};
use bpaf::Bpaf;
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{
    CoreEnvironmentError,
    Environment,
    EnvironmentError,
    FLOX_ENV_CACHE_VAR,
    FLOX_ENV_DESCRIPTION_VAR,
    FLOX_ENV_PROJECT_VAR,
};
use flox_rust_sdk::models::lockfile::LockedManifestError;
use flox_rust_sdk::models::pkgdb::{error_codes, CallPkgDbError, PkgDbError};
use indoc::formatdoc;
use itertools::Itertools;
use log::{debug, warn};

use super::{environment_select, EnvironmentSelect, UninitializedEnvironment};
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

        let prompt_color_1 = env::var("FLOX_PROMPT_COLOR_1")
            .unwrap_or(utils::colors::INDIGO_400.to_ansi256().to_string());
        let prompt_color_2 = env::var("FLOX_PROMPT_COLOR_2")
            .unwrap_or(utils::colors::INDIGO_300.to_ansi256().to_string());

        let mut exports = HashMap::from([
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
            // Export FLOX_ENV_DESCRIPTION for this environment and let the
            // activation script take care of tracking active environments
            // and invoking the appropriate script to set the prompt.
            (
                FLOX_ENV_DESCRIPTION_VAR,
                now_active
                    .bare_description()
                    .expect("`bare_description` is infallible"),
            ),
        ]);

        exports.extend(default_nix_env_vars());

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
    fn old_activate_command(
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

    /// Used for `flox activate -- run_args`
    fn activate_command(
        run_args: Vec<String>,
        shell: Shell,
        exports: HashMap<&str, String>,
        activation_path: PathBuf,
    ) -> Result<()> {
        // Previous versions of pkgdb rendered activation scripts into a
        // subdirectory called "activate", but now that path is occupied by
        // the activation script itself. The new activation scripts are in a
        // subdirectory called "activate.d". If we find that the "activate"
        // path is a directory, we assume it's the old style and invoke the
        // old_activate_command function.
        let activate_path = activation_path.join("activate");
        if activate_path.is_dir() {
            return Self::old_activate_command(run_args, shell, exports, activation_path);
        }

        let mut command = Command::new(activate_path);
        command.args(run_args);
        command.envs(exports);

        debug!("running activation command: {:?}", command);

        // exec should never return
        Err(command.exec().into())
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn old_activate_interactive(
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
                    .env(
                        "ZDOTDIR",
                        activation_path.join("activate.d").join("zdotdir"),
                    )
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
        // Previous versions of pkgdb rendered activation scripts into a
        // subdirectory called "activate", but now that path is occupied by
        // the activation script itself. The new activation scripts are in a
        // subdirectory called "activate.d". If we find that the "activate"
        // path is a directory, we assume it's the old style and invoke the
        // old_activate_interactive function.
        let activate_path = activation_path.join("activate");
        if activate_path.is_dir() {
            return Self::old_activate_interactive(shell, exports, activation_path, now_active);
        }

        let mut command = Command::new(activate_path);
        command.env("FLOX_SHELL", shell.exe_path());
        command.envs(exports);

        debug!("running activation command: {:?}", command);

        let message = formatdoc! {"
                You are now using the environment {}.
                To stop using this environment, type 'exit'\n", now_active.message_description()?};
        message::updated(message);

        // exec should never return
        Err(command.exec().into())
    }

    /// Used for `eval "$(flox activate)"`
    fn old_activate_in_place(
        shell: &Shell,
        exports: &HashMap<&str, String>,
        activation_path: &Path,
    ) {
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

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(shell: &Shell, exports: &HashMap<&str, String>, activation_path: &Path) {
        // Previous versions of pkgdb rendered activation scripts into a
        // subdirectory called "activate", but now that path is occupied by
        // the activation script itself. The new activation scripts are in a
        // subdirectory called "activate.d". If we find that the "activate"
        // path is a directory, we assume it's the old style and invoke the
        // old_activate_in_place function.
        let activate_path = activation_path.join("activate");
        if activate_path.is_dir() {
            return Self::old_activate_in_place(shell, exports, activation_path);
        }

        let mut command = Command::new(&activate_path);
        command.env("FLOX_SHELL", shell.exe_path());
        command.envs(exports);

        debug!("running activation command: {:?}", command);

        let output = command.output().expect("failed to run activation script");
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        // XXX BUG TODO: this is not correct, we need to know the value of
        // $FLOX_SHELL in order to know the correct syntax for exporting
        // variables in the local shell dialect. Turn this into a function
        // that can do that.
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
            {output}
            unset FLOX_SOURCED_FROM_SHELL_RC
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");
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
