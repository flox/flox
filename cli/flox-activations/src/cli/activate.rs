use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow};
use clap::Args;
use flox_core::activate_data::ActivateData;
use flox_core::activations::activations_json_path;
use flox_core::shell::Shell;
use flox_core::util::default_nix_env_vars;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use time::{Duration, OffsetDateTime};

use super::StartOrAttachArgs;
use super::start_or_attach::wait_for_activation_ready_and_optionally_attach_pid;

#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// Path to JSON file containing activation data
    #[arg(long)]
    pub activate_data: PathBuf,
}

pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_ACTIVE_ENVIRONMENTS_VAR: &str = "_FLOX_ACTIVE_ENVIRONMENTS";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";
/// This variable is used in tests to override what path to use for the socket.
pub const FLOX_SERVICES_SOCKET_OVERRIDE_VAR: &str = "_FLOX_SERVICES_SOCKET_OVERRIDE";

// TODO
pub const FLOX_RUNTIME_DIR_VAR: &str = "FLOX_RUNTIME_DIR";
pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";

impl ActivateArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let data: ActivateData = serde_json::from_str(&contents)?;

        fs::remove_file(&self.activate_data)?;

        let (attach, activation_state_dir, activation_id) = StartOrAttachArgs {
            pid: std::process::id() as i32,
            flox_env: PathBuf::from(&data.env),
            store_path: data.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&data.flox_runtime_dir),
        }
        .handle()?;

        if !attach {
            let mut start_command = Self::assemble_command(data.clone());
            start_command.arg("--mode").arg("start");
            start_command
                .arg("--activation-state-dir")
                .arg(activation_state_dir.to_string_lossy().to_string());
            start_command.arg("--activation-id").arg(&activation_id);
            start_command
                .stderr(Stdio::null())
                .stdout(Stdio::null())
                .stdin(Stdio::null())
                .spawn()?;

            let attach_expiration = OffsetDateTime::now_utc() + Duration::seconds(10);
            wait_for_activation_ready_and_optionally_attach_pid(
                &activations_json_path(&data.flox_runtime_dir, data.env.clone()),
                &data.flox_activate_store_path,
                attach_expiration,
                None,
            )?;
        }
        let mut command = Self::assemble_command(data.clone());
        // Pass down the activation mode
        command.arg("--mode").arg(data.mode);
        command
            .arg("--activation-state-dir")
            .arg(activation_state_dir.to_string_lossy().to_string());
        command.arg("--activation-id").arg(activation_id);

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if data.in_place {
            Self::activate_in_place(command, data.shell)?;

            return Ok(());
        }

        // These functions will only return if exec fails
        if data.interactive {
            Self::activate_interactive(command)
        } else {
            Self::activate_command(command, data.run_args, data.is_ephemeral)
        }
    }

    fn assemble_command(data: ActivateData) -> Command {
        let mut exports = HashMap::from([
            (FLOX_ACTIVE_ENVIRONMENTS_VAR, data.flox_active_environments),
            (FLOX_ENV_LOG_DIR_VAR, data.flox_env_log_dir),
            ("FLOX_PROMPT_COLOR_1", data.prompt_color_1),
            ("FLOX_PROMPT_COLOR_2", data.prompt_color_2),
            // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
            // which may be ""
            (FLOX_PROMPT_ENVIRONMENTS_VAR, data.flox_prompt_environments),
            ("_FLOX_SET_PROMPT", data.set_prompt.to_string()),
            (
                "_FLOX_ACTIVATE_STORE_PATH",
                data.flox_activate_store_path.clone(),
            ),
            (
                // TODO: we should probably figure out a more consistent way to
                // pass this since it's also passed for `flox build`
                FLOX_RUNTIME_DIR_VAR,
                data.flox_runtime_dir.clone(),
            ),
            ("_FLOX_ENV_CUDA_DETECTION", data.flox_env_cuda_detection),
            (
                FLOX_ACTIVATE_START_SERVICES_VAR,
                data.flox_activate_start_services.to_string(),
            ),
            (FLOX_SERVICES_SOCKET_VAR, data.flox_services_socket),
        ]);

        if let Some(services_to_start) = data.flox_services_to_start {
            exports.insert(FLOX_SERVICES_TO_START_VAR, services_to_start);
        }

        exports.extend(default_nix_env_vars());

        let activate_path = data.interpreter_path.join("activate");
        let mut command = Command::new(activate_path);
        command.envs(exports);

        command.arg("--env").arg(&data.env);
        command
            .arg("--env-project")
            .arg(data.env_project.to_string_lossy().to_string());
        command
            .arg("--env-cache")
            .arg(data.env_cache.to_string_lossy().to_string());
        command.arg("--env-description").arg(data.env_description);

        command
            .arg("--watchdog")
            .arg(data.watchdog.to_string_lossy().to_string());

        command.arg("--shell").arg(data.shell.exe_path());

        command
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
    fn activate_in_place(mut command: Command, shell: Shell) -> Result<()> {
        debug!("running activation command: {:?}", command);

        let output = command
            .output()
            .context("failed to run activation script")?;
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

        Ok(())
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_run_args() {
        assert_eq!(
            ActivateArgs::quote_run_args(&["a b".to_string(), '"'.to_string()]),
            r#""a b" "\"""#
        )
    }
}
