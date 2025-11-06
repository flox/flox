use std::collections::HashMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::{FLOX_ACTIVE_ENVIRONMENTS_VAR, FLOX_RUNTIME_DIR_VAR};
use flox_core::shell::ShellWithPath;
use flox_core::util::default_nix_env_vars;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;

use crate::shell_gen::Shell;

#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// Path to JSON file containing activation data
    #[arg(long)]
    pub activate_data: PathBuf,
}

pub const FLOX_ENV_LOG_DIR_VAR: &str = "_FLOX_ENV_LOG_DIR";
pub const FLOX_PROMPT_ENVIRONMENTS_VAR: &str = "FLOX_PROMPT_ENVIRONMENTS";
/// This variable is used to communicate what socket to use to the activate
/// script.
pub const FLOX_SERVICES_SOCKET_VAR: &str = "_FLOX_SERVICES_SOCKET";

pub const FLOX_SERVICES_TO_START_VAR: &str = "_FLOX_SERVICES_TO_START";
pub const FLOX_ACTIVATE_START_SERVICES_VAR: &str = "FLOX_ACTIVATE_START_SERVICES";

impl ActivateArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let data: ActivateCtx = serde_json::from_str(&contents)?;

        fs::remove_file(&self.activate_data)?;

        let activate_script_command = Self::assemble_command_for_activate_script(data.clone());
        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if data.invocation_type == InvocationType::InPlace {
            Self::activate_in_place(activate_script_command, data.shell)?;

            return Ok(());
        }

        // These functions will only return if exec fails
        if data.invocation_type == InvocationType::Interactive {
            Self::activate_interactive(activate_script_command)
        } else {
            Self::activate_command(activate_script_command, data.run_args)
        }
    }

    /// Used for `flox activate -- run_args`
    fn activate_command(mut activate_script_command: Command, run_args: Vec<String>) -> Result<()> {
        // The activation script works like a shell in that it accepts the "-c"
        // flag which takes exactly one argument to be passed verbatim to the
        // userShell invocation. Take this opportunity to combine these args
        // safely, and *exactly* as the user provided them in argv.
        activate_script_command
            .arg("-c")
            .arg(Self::quote_run_args(&run_args));

        debug!("running activation command: {:?}", activate_script_command);

        // exec should never return
        Err(activate_script_command.exec().into())
    }

    /// Activate the environment interactively by spawning a new shell
    /// and running the respective activation scripts.
    ///
    /// This function should never return as it replaces the current process
    fn activate_interactive(mut activate_script_command: Command) -> Result<()> {
        debug!("running activation command: {:?}", activate_script_command);

        // exec should never return
        Err(activate_script_command.exec().into())
    }

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(mut activate_script_command: Command, shell: ShellWithPath) -> Result<()> {
        debug!("running activation command: {:?}", activate_script_command);

        let output = activate_script_command
            .output()
            .context("failed to run activation script")?;
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        let shell: Shell = shell.into();
        // Render the exports in the correct shell dialect.
        let mut exports_rendered = activate_script_command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|v| {
                    (
                        key.to_string_lossy(),
                        shell_escape::escape(v.to_string_lossy()),
                    )
                })
            })
            .map(|(key, value)| shell.export_var(key, value))
            .join(";\n");
        exports_rendered.push(';');

        let script = formatdoc! {"
            {exports_rendered}
            {output}
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");

        Ok(())
    }

    fn assemble_command_for_activate_script(data: ActivateCtx) -> Command {
        let mut exports = HashMap::from([
            (FLOX_ACTIVE_ENVIRONMENTS_VAR, data.flox_active_environments),
            (FLOX_ENV_LOG_DIR_VAR, data.flox_env_log_dir),
            ("FLOX_PROMPT_COLOR_1", data.prompt_color_1),
            ("FLOX_PROMPT_COLOR_2", data.prompt_color_2),
            // Set `FLOX_PROMPT_ENVIRONMENTS` to the constructed prompt string,
            // which may be ""
            (FLOX_PROMPT_ENVIRONMENTS_VAR, data.flox_prompt_environments),
            ("_FLOX_SET_PROMPT", data.set_prompt.to_string()),
            ("_FLOX_ACTIVATE_STORE_PATH", data.flox_activate_store_path),
            (
                // TODO: we should probably figure out a more consistent way to
                // pass this since it's also passed for `flox build`
                FLOX_RUNTIME_DIR_VAR,
                data.flox_runtime_dir,
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

        // Pass down the activation mode
        command.arg("--mode").arg(data.mode);

        command
            .arg("--watchdog")
            .arg(data.watchdog_bin.to_string_lossy().to_string());

        command.arg("--shell").arg(data.shell.exe_path());

        command
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
