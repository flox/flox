mod activate_script_builder;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use activate_script_builder::assemble_command_for_activate_script;
use anyhow::{Context, Result};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::shell::ShellWithPath;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;

#[derive(Debug, Args)]
pub struct ActivateArgs {
    /// Path to JSON file containing activation data
    #[arg(long)]
    pub activate_data: PathBuf,

    /// Additional arguments used to provide a command to run.
    /// NOTE: this is only relevant for containerize activations.
    #[arg(allow_hyphen_values = true)]
    pub cmd: Option<Vec<String>>,
}

impl ActivateArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let mut data: ActivateCtx = serde_json::from_str(&contents)?;

        if data.remove_after_reading {
            fs::remove_file(&self.activate_data)?;
        }

        // In the case of containerize, you can't bake-in the invocation type or the
        // `run_args`, so you need to do that detection at runtime. Here we do that
        // by modifying the `ActivateCtx` passed to us in the container's
        // EntryPoint.
        let run_args = self
            .cmd
            .as_ref()
            .or(Some(&data.run_args))
            .and_then(|args| if args.is_empty() { None } else { Some(args) });

        match (data.invocation_type.as_ref(), run_args) {
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, None) => data.invocation_type = Some(InvocationType::Interactive),
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, Some(args)) => {
                data.invocation_type = Some(InvocationType::Command);
                data.run_args = args.clone();
            },
            // The following two cases are normal shell activations, and don't need
            // to modify the activation context.
            (Some(_), None) => {},
            (Some(_), Some(_)) => {},
        }
        // For any case where `invocation_type` is None, we should have detected that above
        // and set it to Some.
        let invocation_type = data
            .invocation_type
            .expect("invocation type should have been some");

        let activate_script_command = assemble_command_for_activate_script(data.clone());

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if invocation_type == InvocationType::InPlace {
            Self::activate_in_place(activate_script_command, data.shell)?;

            return Ok(());
        }

        // These functions will only return if exec fails
        if invocation_type == InvocationType::Interactive {
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

        // Render the exports in the correct shell dialect.
        let exports_rendered = activate_script_command
            .get_envs()
            .filter_map(|(key, value)| {
                value.map(|v| {
                    (
                        key.to_string_lossy(),
                        shell_escape::escape(v.to_string_lossy()),
                    )
                })
            })
            // TODO: we should use a method on Shell here, possibly using
            // shell_escape in the Shell method?
            // But not quoting here is intentional because we already use shell_escape
            .map(|(key, value)| match shell {
                ShellWithPath::Bash(_) => format!("export {key}={value};",),
                ShellWithPath::Fish(_) => format!("set -gx {key} {value};",),
                ShellWithPath::Tcsh(_) => format!("setenv {key} {value};",),
                ShellWithPath::Zsh(_) => format!("export {key}={value};",),
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
