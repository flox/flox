use std::collections::HashMap;
use std::fmt::Display;
use std::io::stdout;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::LazyLock;
use std::{env, fs};

use anyhow::{anyhow, bail, Context, Result};
use bpaf::Bpaf;
use crossterm::tty::IsTty;
use flox_rust_sdk::flox::{Flox, DEFAULT_NAME};
use flox_rust_sdk::models::environment::{
    ConcreteEnvironment,
    Environment,
    EnvironmentError,
    FLOX_ACTIVE_ENVIRONMENTS_VAR,
    FLOX_ENV_LOG_DIR_VAR,
    FLOX_PROMPT_ENVIRONMENTS_VAR,
    FLOX_SERVICES_SOCKET_VAR,
};
use flox_rust_sdk::models::manifest::typed::Inner;
use flox_rust_sdk::providers::build::FLOX_RUNTIME_DIR_VAR;
use flox_rust_sdk::providers::services::shutdown_process_compose_if_all_processes_stopped;
use flox_rust_sdk::providers::upgrade_checks::UpgradeInformationGuard;
use flox_rust_sdk::utils::logging::traceable_path;
use indoc::{formatdoc, indoc};
use itertools::Itertools;
use tracing::{debug, warn};

use super::services::ServicesEnvironment;
use super::{
    activated_environments,
    environment_select,
    EnvironmentSelect,
    UninitializedEnvironment,
};
use crate::commands::check_for_upgrades::spawn_detached_check_for_upgrades_process;
use crate::commands::services::ServicesCommandsError;
use crate::commands::{ensure_environment_trust, EnvironmentSelectError};
use crate::config::{Config, EnvironmentPromptConfig};
use crate::utils::openers::Shell;
use crate::utils::{default_nix_env_vars, message};
use crate::{environment_subcommand_metric, subcommand_metric, utils};

pub static INTERACTIVE_BASH_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(
        env::var("INTERACTIVE_BASH_BIN").unwrap_or(env!("INTERACTIVE_BASH_BIN").to_string()),
    )
});
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";
pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub static WATCHDOG_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(env::var("WATCHDOG_BIN").unwrap_or(env!("WATCHDOG_BIN").to_string()))
});
pub static FLOX_INTERPRETER: LazyLock<PathBuf> = LazyLock::new(|| {
    PathBuf::from(env::var("FLOX_INTERPRETER").unwrap_or(env!("FLOX_INTERPRETER").to_string()))
});

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Mode {
    #[default]
    Dev,
    Run,
}

impl Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Dev => write!(f, "dev"),
            Mode::Run => write!(f, "run"),
        }
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "dev" => Ok(Mode::Dev),
            "run" => Ok(Mode::Run),
            _ => Err(anyhow!("'{s}' is not a valid activation mode")),
        }
    }
}

#[derive(Bpaf, Clone)]
pub struct Activate {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    pub environment: EnvironmentSelect,

    /// Trust a remote environment temporarily for this activation
    #[bpaf(long, short)]
    pub trust: bool,

    /// Print an activation script to stdout instead of spawning a subshell
    #[bpaf(long("print-script"), short, hide)]
    pub print_script: bool,

    /// Whether to start services when activating the environment
    #[bpaf(long, short)]
    pub start_services: bool,

    /// Use the interpreter bundled with the environment instead of the
    /// interpreter bundled with the CLI.
    #[bpaf(long, hide)]
    pub use_fallback_interpreter: bool,

    /// Whether to activate in "dev" mode or "run" mode, the difference being
    /// that "dev" mode will set environment variables necessary for
    /// development, whereas "run" will simply put executables in PATH.
    #[bpaf(short, long)]
    pub mode: Option<Mode>,

    /// Command to run interactively in the context of the environment
    #[bpaf(positional("cmd"), strict, many)]
    pub run_args: Vec<String>,
}

impl Activate {
    pub async fn handle(self, mut config: Config, flox: Flox) -> Result<()> {
        environment_subcommand_metric!(
            "activate",
            self.environment,
            start_services = self.start_services
        );

        let mut concrete_environment = match self.environment.to_concrete_environment(&flox) {
            Ok(concrete_environment) => concrete_environment,
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
            {e}

            Create an environment with 'flox init'"
                })
            },
            Err(EnvironmentSelectError::Anyhow(e)) => Err(e)?,
            Err(e) => Err(e)?,
        };

        if let ConcreteEnvironment::Remote(ref env) = concrete_environment {
            if !self.trust {
                ensure_environment_trust(&mut config, &flox, env).await?;
            }
        }

        if config.flox.upgrade_notifications.unwrap_or(true) {
            // Read the results of a previous upgrade check
            // and print a message if an upgrade is available.
            notify_upgrade_if_available(&flox, &mut concrete_environment)?;
        } else {
            debug!("Upgrade notification disabled");
        }

        // Spawn a detached process to check for upgrades in the background.
        let environment =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment)?;
        spawn_detached_check_for_upgrades_process(
            &environment,
            None,
            &concrete_environment.log_path()?,
            None,
        )?;

        self.activate(config, flox, concrete_environment, false, &[])
            .await
    }

    /// This function contains the bulk of the implementation for
    /// Activate::handle,
    /// but it allows us to create an activation for use by `services start` or
    /// `services-restart`.
    ///
    /// If self.start_services is true and services_to_start is empty, all
    /// services will be started.
    // TODO: there's probably a cleaner way to extract the functionality we need
    // for start and restart,
    // but for now just hack through the is_ephemeral bool.
    pub async fn activate(
        self,
        config: Config,
        flox: Flox,
        mut concrete_environment: ConcreteEnvironment,
        is_ephemeral: bool,
        services_to_start: &[String],
    ) -> Result<()> {
        let now_active =
            UninitializedEnvironment::from_concrete_environment(&concrete_environment)?;

        let environment = concrete_environment.dyn_environment_ref_mut();

        let in_place = self.print_script || (!stdout().is_tty() && self.run_args.is_empty());
        let interactive = !in_place && self.run_args.is_empty();
        let mode = self.mode.clone().unwrap_or_default();

        // Don't spin in bashrcs and similar contexts
        let rendered_env_path_result = environment.rendered_env_links(&flox);

        let rendered_env_path = match rendered_env_path_result {
            Err(EnvironmentError::Core(err)) if err.is_incompatible_system_error() => {
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

        let mode_link_path = match mode {
            Mode::Dev => &rendered_env_path.development.clone(),
            Mode::Run => &rendered_env_path.runtime.clone(),
        };
        let store_path = fs::read_link(mode_link_path).with_context(|| {
            format!(
                "a symlink at {} was just created and should still exist",
                mode_link_path.display()
            )
        })?;

        let interpreter_path = if self.use_fallback_interpreter {
            let path = rendered_env_path.development.to_path_buf();
            tracing::debug!(
                interpreter = "stored",
                path = traceable_path(&path),
                "setting interpreter"
            );
            path
        } else {
            let path = FLOX_INTERPRETER.clone();
            tracing::debug!(
                interpreter = "bundled",
                path = traceable_path(&path),
                "setting interpreter"
            );
            path
        };

        // Must come after getting an activation path to prevent premature
        // locking or migration. It must also not be evaluated inline with the
        // macro or we'll leak TRACE logs for reasons unknown.
        let lockfile_version = environment.lockfile(&flox)?.version();
        subcommand_metric!("activate#version", lockfile_version = lockfile_version);

        // read the currently active environments from the environment
        let mut flox_active_environments = activated_environments();

        // Detect if the current environment is already active
        // For in-place and command (but not ephemeral) activations, if the
        // environment is already active, we only want to re-run profile scripts
        let profile_only = if flox_active_environments.is_active(&now_active) {
            debug!(
                "Environment is already active: environment={}. Not adding to active environments",
                now_active.bare_description()?
            );
            if interactive {
                return Err(anyhow!(
                    "Environment {} is already active",
                    now_active.message_description()?
                ));
            }
            !is_ephemeral
        } else {
            // Add to _FLOX_ACTIVE_ENVIRONMENTS so we can detect what environments are active.
            flox_active_environments.set_last_active(now_active.clone());
            false
        };

        // Determine values for `set_prompt` and `hide_default_prompt`, taking
        // deprecated `shell_prompt` into account
        let (set_prompt, hide_default_prompt) = match (
            config.flox.set_prompt,
            config.flox.hide_default_prompt,
            config.flox.shell_prompt,
        ) {
            (None, None, Some(EnvironmentPromptConfig::ShowAll)) => (true, false),
            (None, None, Some(EnvironmentPromptConfig::HideDefault)) => (true, true),
            (None, None, Some(EnvironmentPromptConfig::HideAll)) => (false, false),
            (Some(_), _, Some(_)) | (_, Some(_), Some(_)) => bail!(indoc! {"
                'shell_prompt' has been deprecated and cannot be set when 'set_prompt' or
                'hide_default_prompt' is set.

                Remove 'shell_prompt' with 'flox config --delete shell_prompt'
            "}),
            (set_prompt, hide_default_prompt, _) => (
                set_prompt.unwrap_or(true),
                hide_default_prompt.unwrap_or(true),
            ),
        };

        // We don't have access to the current PS1 (it's not exported), so we
        // can't modify it. Instead set FLOX_PROMPT_ENVIRONMENTS and let the
        // activation script set PS1 based on that.
        let flox_prompt_environments =
            Self::make_prompt_environments(hide_default_prompt, &flox_active_environments);

        let prompt_color_1 = env::var("FLOX_PROMPT_COLOR_1")
            .unwrap_or(utils::colors::INDIGO_400.to_ansi256().to_string());
        let prompt_color_2 = env::var("FLOX_PROMPT_COLOR_2")
            .unwrap_or(utils::colors::INDIGO_300.to_ansi256().to_string());

        let mut exports = HashMap::from([
            (
                FLOX_ACTIVE_ENVIRONMENTS_VAR,
                flox_active_environments.to_string(),
            ),
            (
                FLOX_ENV_LOG_DIR_VAR,
                environment.log_path()?.to_string_lossy().to_string(),
            ),
            ("FLOX_PROMPT_COLOR_1", prompt_color_1),
            ("FLOX_PROMPT_COLOR_2", prompt_color_2),
            // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
            // which may be ""
            (FLOX_PROMPT_ENVIRONMENTS_VAR, flox_prompt_environments),
            ("_FLOX_SET_PROMPT", set_prompt.to_string()),
            (
                "_FLOX_ACTIVATE_STORE_PATH",
                store_path.to_string_lossy().to_string(),
            ),
            (
                // TODO: we should probably figure out a more consistent way to
                // pass this since it's also passed for `flox build`
                FLOX_RUNTIME_DIR_VAR,
                flox.runtime_dir.to_string_lossy().to_string(),
            ),
            ("_FLOX_ACTIVATION_PROFILE_ONLY", profile_only.to_string()),
        ]);

        if is_ephemeral && !services_to_start.is_empty() {
            exports.insert(
                FLOX_SERVICES_TO_START_VAR,
                // Store JSON in an env var because bash doesn't
                // support storing arrays in env vars
                serde_json::to_string(&services_to_start)?,
            );
        }

        let socket_path = environment.services_socket_path(&flox)?;
        let manifest = environment.manifest(&flox)?;
        exports.insert(
            "_FLOX_ENV_CUDA_DETECTION",
            match manifest.options.cuda_detection {
                Some(false) => "0", // manifest opts-out
                _ => "1",           // default to enabling CUDA
            }
            .to_string(),
        );

        if self.start_services {
            ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;

            if manifest.services.inner().is_empty() {
                message::warning(ServicesCommandsError::NoDefinedServices);
            } else if manifest
                .services
                .copy_for_system(&flox.system)
                .inner()
                .is_empty()
            {
                message::warning(ServicesCommandsError::NoDefinedServicesForSystem {
                    system: flox.system.clone(),
                });
            }
        }

        let should_have_services = self.start_services
            && !manifest
                .services
                .copy_for_system(&flox.system)
                .inner()
                .is_empty();
        let start_new_process_compose = should_have_services
            && if socket_path.exists() {
                // Returns `Ok(true)` if `process-compose` was shutdown
                shutdown_process_compose_if_all_processes_stopped(&socket_path)?
            } else {
                true
            };
        tracing::debug!(
            should_have_services,
            start_new_process_compose,
            "setting service variables"
        );
        exports.insert(
            FLOX_ACTIVATE_START_SERVICES_VAR,
            start_new_process_compose.to_string(),
        );
        exports.insert(
            FLOX_SERVICES_SOCKET_VAR,
            socket_path.to_string_lossy().to_string(),
        );
        if should_have_services && !start_new_process_compose {
            message::warning("Skipped starting services, services are already running");
        }

        exports.extend(default_nix_env_vars());

        let activate_path = interpreter_path.join("activate");
        let mut command = Command::new(activate_path);
        command.envs(exports);

        // Don't rely on FLOX_ENV in the environment when we explicitly know
        // what it should be. This is necessary for nested activations where an
        // outer export of FLOX_ENV would be inherited by the inner activation.
        command
            .arg("--env")
            .arg(mode_link_path.to_string_lossy().to_string());
        command
            .arg("--env-project")
            .arg(environment.project_path()?.to_string_lossy().to_string());
        command
            .arg("--env-cache")
            .arg(environment.cache_path()?.to_string_lossy().to_string());
        command.arg("--env-description").arg(
            now_active
                .bare_description()
                .expect("`bare_description` is infallible"),
        );

        // Pass down the activation mode
        command.arg("--mode").arg(mode.to_string());

        command
            .arg("--watchdog")
            .arg(WATCHDOG_BIN.to_string_lossy().to_string());

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if in_place {
            let shell = Self::detect_shell_for_in_place()?;
            command.arg("--shell").arg(shell.exe_path());
            Self::activate_in_place(command, shell);

            return Ok(());
        }

        let shell = Self::detect_shell_for_subshell();
        command.arg("--shell").arg(shell.exe_path());
        // These functions will only return if exec fails
        if interactive {
            Self::activate_interactive(command)
        } else {
            Self::activate_command(command, self.run_args, is_ephemeral)
        }
    }

    /// Used for `flox activate -- run_args`
    fn activate_command(
        mut command: Command,
        run_args: Vec<String>,
        is_ephemeral: bool,
    ) -> Result<()> {
        // The activation script works like a shell in that it accepts the "-c"
        // flag which takes exactly one argument to be passed verbatim to the
        // userShell invocation. Take this opportunity to combine these args
        // safely, and *exactly* as the user provided them in argv.
        command.arg("-c").arg(Self::quote_run_args(&run_args));

        debug!("running activation command: {:?}", command);

        if is_ephemeral {
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                Err(anyhow!(
                    "failed to run activation script: {}",
                    String::from_utf8_lossy(&output.stderr)
                ))?;
            }
            Ok(())
        } else {
            // exec should never return
            Err(command.exec().into())
        }
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn activate_interactive(mut command: Command) -> Result<()> {
        debug!("running activation command: {:?}", command);

        // exec should never return
        Err(command.exec().into())
    }

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(mut command: Command, shell: Shell) {
        debug!("running activation command: {:?}", command);

        let output = command.output().expect("failed to run activation script");
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        // Render the exports in the correct shell dialect.
        let exports_rendered = command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|v| {
                    (
                        key.to_string_lossy(),
                        shell_escape::escape(v.to_string_lossy()),
                    )
                })
            })
            .map(|(key, value)| match shell {
                Shell::Bash(_) => format!("export {key}={value};",),
                Shell::Fish(_) => format!("set -gx {key} {value};",),
                Shell::Tcsh(_) => format!("setenv {key} {value};",),
                Shell::Zsh(_) => format!("export {key}={value};",),
            })
            .join("\n");

        let script = formatdoc! {"
            {exports_rendered}
            {output}
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");
    }

    /// Quote run args so that words don't get split,
    /// but don't escape all characters.
    ///
    /// To do this we escape '"' and '`',
    /// but we don't escape anything else.
    /// We want '$' for example to be expanded by the shell.
    fn quote_run_args(run_args: &[String]) -> String {
        run_args
            .iter()
            .map(|arg| {
                if arg.contains(' ') || arg.contains('"') || arg.contains('`') {
                    format!(r#""{}""#, arg.replace('"', r#"\""#).replace('`', r#"\`"#))
                } else {
                    arg.to_string()
                }
            })
            .join(" ")
    }

    /// Detect the shell to use for activation
    ///
    /// Used to determine shell for
    /// `flox activate` and `flox activate -- CMD`
    ///
    /// Returns the first shell found in the following order:
    /// 1. FLOX_SHELL environment variable
    /// 2. SHELL environment variable
    /// 3. Parent process shell
    /// 4. Default to bash bundled with flox
    fn detect_shell_for_subshell() -> Shell {
        Self::detect_shell_for_subshell_with(Shell::detect_from_parent_process)
    }

    /// Utility method for testing implementing the logic of shell detection
    /// for subshells, generically over a parent shell detection function.
    fn detect_shell_for_subshell_with(parent_shell_fn: impl Fn() -> Result<Shell>) -> Shell {
        Shell::detect_from_env("FLOX_SHELL")
            .or_else(|err| {
                debug!("Failed to detect shell from FLOX_SHELL: {err}");
                Shell::detect_from_env("SHELL")
            })
            .or_else(|err| {
                debug!("Failed to detect shell from SHELL: {err}");
                parent_shell_fn()
            })
            .unwrap_or_else(|err| {
                debug!("Failed to detect shell from parent process: {err}");
                warn!(
                    "Failed to detect shell from environment or parent process. Defaulting to bash"
                );
                Shell::Bash(INTERACTIVE_BASH_BIN.clone())
            })
    }

    /// Detect the shell to use for in-place activation
    ///
    /// Used to determine shell for `eval "$(flox activate)"`,
    /// `flox activate --print-script`, and
    /// when adding activation of a default environment to RC files.
    pub(crate) fn detect_shell_for_in_place() -> Result<Shell> {
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

    /// Construct the environment list for the shell prompt
    ///
    /// [`None`] if the prompt is disabled, or filters removed all components.
    fn make_prompt_environments(
        hide_default_prompt: bool,
        flox_active_environments: &super::ActiveEnvironments,
    ) -> String {
        let prompt_envs: Vec<_> = flox_active_environments
            .iter()
            .filter_map(|env| {
                if hide_default_prompt && env.name().as_ref() == DEFAULT_NAME {
                    return None;
                }
                Some(
                    env.bare_description()
                        .expect("`bare_description` is infallible"),
                )
            })
            .collect();

        prompt_envs.join(" ")
    }
}

/// Notify the user of available upgrades
///
/// Upon activation flox will start a detached process to check for upgrades.
/// Future activations will be able to read the upgrade information from a file
/// and notify the user if there are any upgrades available using this function.
/// See [spawn_detached_check_for_upgrades_process] for more information
/// on the upgrade check process.
///
/// This function reads the upgrade information for a given environment,
/// and prints a message to the user if the upgrade information is still applicable
/// to the current environment -- based on the same lockfile
/// and indicating that upgrades are available -- and the environment isn't
/// already active, to prevent immediate duplications.
///
/// There is no refractory period for upgrade notifications,
/// i.e. we message _every_ time this function is called by `flox activate`.
/// The motivation for this is to provide deterministic behavior,
/// compared to comparatively random display of upgrade messages every hour or so.
/// For example, when a user activates an environment and sees a message,
/// but doesn't act on it, they should see the message again next time they activate,
/// so they are not wondering whether upgrades may have been applied automatically.
/// To make this less annoying, we tried to make the message as unobtrusive as possible.
fn notify_upgrade_if_available(flox: &Flox, environment: &mut ConcreteEnvironment) -> Result<()> {
    let current_environment = UninitializedEnvironment::from_concrete_environment(environment)?;
    let active_environments = activated_environments();
    if active_environments.is_active(&current_environment) {
        debug!("Not notifying user of upgrade, environment is already active");
        return Ok(());
    }

    let upgrade_guard = UpgradeInformationGuard::read_in(environment.cache_path()?)?;

    let Some(info) = upgrade_guard.info() else {
        debug!("Not notifying user of upgrade, no upgrade information available");
        return Ok(());
    };

    let current_lockfile = environment.lockfile(flox)?;

    if Some(current_lockfile) != info.result.old_lockfile {
        // todo: delete the info file?
        debug!("Not notifying user of upgrade, lockfile has changed since last check");
        return Ok(());
    }

    let diff = info.result.diff();
    if diff.is_empty() {
        debug!("Not notifying user of upgrade, no changes in lockfile");
        return Ok(());
    }

    let description =
        UninitializedEnvironment::from_concrete_environment(environment)?.message_description()?;

    // Update this message in flox-config.md if you change it here
    let message = formatdoc! {"
        ℹ️  Upgrades are available for packages in {description}.
        Use 'flox upgrade' to get the latest.
    "};

    message::plain(message);

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use flox_rust_sdk::models::environment::{DotFlox, EnvironmentPointer, PathPointer};

    use super::*;
    use crate::commands::ActiveEnvironments;

    static DEFAULT_ENV: LazyLock<UninitializedEnvironment> = LazyLock::new(|| {
        UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from(""),
            pointer: EnvironmentPointer::Path(PathPointer::new("default".parse().unwrap())),
        })
    });

    static NON_DEFAULT_ENV: LazyLock<UninitializedEnvironment> = LazyLock::new(|| {
        UninitializedEnvironment::DotFlox(DotFlox {
            path: PathBuf::from(""),
            pointer: EnvironmentPointer::Path(PathPointer::new("wichtig".parse().unwrap())),
        })
    });

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
            let shell = Activate::detect_shell_for_subshell_with(|| unreachable!());
            assert_eq!(shell, Shell::Bash("/shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_SET, SHELL_SET], || {
            let shell = Activate::detect_shell_for_subshell_with(|| unreachable!());
            assert_eq!(shell, Shell::Bash("/flox_shell/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = Activate::detect_shell_for_subshell_with(PARENT_DETECTED);
            assert_eq!(shell, Shell::Bash("/parent/bash".into()));
        });

        temp_env::with_vars([FLOX_SHELL_UNSET, SHELL_UNSET], || {
            let shell = Activate::detect_shell_for_subshell_with(PARENT_UNDETECTED);
            assert_eq!(shell, Shell::Bash(INTERACTIVE_BASH_BIN.clone()));
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

    #[test]
    fn test_shell_prompt_empty_without_active_environments() {
        let active_environments = ActiveEnvironments::default();
        let prompt = Activate::make_prompt_environments(false, &active_environments);

        assert_eq!(prompt, "");
    }

    #[test]
    fn test_shell_prompt_default() {
        let mut active_environments = ActiveEnvironments::default();
        active_environments.set_last_active(DEFAULT_ENV.clone());

        // with `hide_default_prompt = false` we should see the default environment
        let prompt = Activate::make_prompt_environments(false, &active_environments);
        assert_eq!(prompt, "default".to_string());

        // with `hide_default_prompt = true` we should not see the default environment
        let prompt = Activate::make_prompt_environments(true, &active_environments);
        assert_eq!(prompt, "");
    }

    #[test]
    fn test_shell_prompt_mixed() {
        let mut active_environments = ActiveEnvironments::default();
        active_environments.set_last_active(DEFAULT_ENV.clone());
        active_environments.set_last_active(NON_DEFAULT_ENV.clone());

        // with `hide_default_prompt = false` we should see the default environment
        let prompt = Activate::make_prompt_environments(false, &active_environments);
        assert_eq!(prompt, "wichtig default".to_string());

        // with `hide_default_prompt = true` we should not see the default environment
        let prompt = Activate::make_prompt_environments(true, &active_environments);
        assert_eq!(prompt, "wichtig".to_string());
    }
}

#[cfg(test)]
mod upgrade_notification_tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_from_env_files;
    use flox_rust_sdk::models::environment::UpgradeResult;
    use flox_rust_sdk::models::lockfile::LockedPackage;
    use flox_rust_sdk::providers::catalog::GENERATED_DATA;
    use flox_rust_sdk::providers::upgrade_checks::UpgradeInformation;
    use flox_rust_sdk::utils::logging::test_helpers::CollectingWriter;
    use time::OffsetDateTime;
    use tracing::Subscriber;
    use tracing_subscriber::filter::FilterFn;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::commands::ActiveEnvironments;

    fn test_subscriber() -> (impl Subscriber, CollectingWriter) {
        let (subscriber, writer) = flox_rust_sdk::utils::logging::test_helpers::test_subscriber();
        let subscriber = subscriber.with(FilterFn::new(|metadata| {
            metadata.target() == "flox::utils::message"
        }));
        (subscriber, writer)
    }

    #[test]
    fn no_notification_printed_if_absent() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrade_if_available(&flox, &mut environment).unwrap();
        });

        let printed = writer.to_string();

        assert!(printed.is_empty(), "printed: {printed}");
    }

    fn write_upgrade_available(flox: &Flox, environment: &mut ConcreteEnvironment) {
        let upgrade_information =
            UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();
        let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

        let mut new_lockfile = environment.lockfile(flox).unwrap();
        for locked_package in new_lockfile.packages.iter_mut() {
            match locked_package {
                LockedPackage::Catalog(locked_package_catalog) => {
                    locked_package_catalog.derivation = "upgraded".to_string()
                },
                LockedPackage::Flake(locked_package_flake) => {
                    locked_package_flake.locked_installable.derivation = "upgraded".to_string()
                },
                LockedPackage::StorePath(_) => {},
            }
        }

        let _ = locked.info_mut().insert(UpgradeInformation {
            last_checked: OffsetDateTime::now_utc(),
            result: UpgradeResult {
                old_lockfile: Some(environment.lockfile(flox).unwrap()),
                new_lockfile,

                store_path: None,
            },
        });

        locked.commit().unwrap();
    }

    #[test]
    fn no_notification_printed_if_already_active() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        let mut active = ActiveEnvironments::default();
        active.set_last_active(
            UninitializedEnvironment::from_concrete_environment(&environment).unwrap(),
        );

        write_upgrade_available(&flox, &mut environment);

        temp_env::with_var(
            FLOX_ACTIVE_ENVIRONMENTS_VAR,
            Some(active.to_string()),
            || {
                tracing::subscriber::with_default(subscriber, || {
                    notify_upgrade_if_available(&flox, &mut environment).unwrap();
                });
            },
        );

        let printed = writer.to_string();

        assert!(printed.is_empty(), "printed: {printed}");
    }

    #[test]
    fn notification_printed_if_present() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        write_upgrade_available(&flox, &mut environment);

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrade_if_available(&flox, &mut environment).unwrap();
        });

        let printed = writer.to_string();

        assert_eq!(printed, formatdoc! {"
            ℹ️  Upgrades are available for packages in 'name'.
            Use 'flox upgrade' to get the latest.

        "});
    }

    #[test]
    fn no_notification_printed_if_outdated() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        {
            let upgrade_information =
                UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();
            let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

            // cause old_lockfile to evaluate as non-equal to the current lockfile
            let mut old_lockfile = environment.lockfile(&flox).unwrap();
            old_lockfile.packages.clear();

            let _ = locked.info_mut().insert(UpgradeInformation {
                last_checked: OffsetDateTime::now_utc(),
                result: UpgradeResult {
                    old_lockfile: Some(old_lockfile),
                    new_lockfile: environment.lockfile(&flox).unwrap(),

                    store_path: None,
                },
            });

            locked.commit().unwrap();
        }

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrade_if_available(&flox, &mut environment).unwrap();
        });

        let printed = writer.to_string();
        assert!(printed.is_empty(), "printed: {printed}");
    }

    #[test]
    fn no_notification_printed_if_no_diff() {
        let (flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));
        let mut environment = ConcreteEnvironment::Path(environment);

        {
            let upgrade_information =
                UpgradeInformationGuard::read_in(environment.cache_path().unwrap()).unwrap();

            let result = UpgradeResult {
                old_lockfile: Some(environment.lockfile(&flox).unwrap()),
                new_lockfile: environment.lockfile(&flox).unwrap(),

                store_path: None,
            };

            assert!(result.diff().is_empty());

            let mut locked = upgrade_information.lock_if_unlocked().unwrap().unwrap();

            let _ = locked.info_mut().insert(UpgradeInformation {
                last_checked: OffsetDateTime::now_utc(),
                result,
            });

            locked.commit().unwrap();
        }

        tracing::subscriber::with_default(subscriber, || {
            notify_upgrade_if_available(&flox, &mut environment).unwrap();
        });

        let printed = writer.to_string();
        assert!(printed.is_empty(), "printed: {printed}");
    }
}
