mod activate_script_builder;
use std::borrow::Cow;
use std::fs::{self, OpenOptions};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use activate_script_builder::{FLOX_ENV_DIRS_VAR, assemble_command_for_activate_script};
use anyhow::{Context, Result, anyhow};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use indoc::formatdoc;
use itertools::Itertools;
use log::debug;
use shell_gen::ShellWithPath;

use super::StartOrAttachArgs;
use crate::cli::activate::activate_script_builder::{
    activate_tracer,
    assemble_command_for_start_script,
    old_cli_envs,
};
use crate::env_diff::EnvDiff;
use crate::gen_rc::bash::{BashStartupArgs, generate_bash_startup_commands};
use crate::gen_rc::fish::{FishStartupArgs, generate_fish_startup_commands};
use crate::gen_rc::tcsh::{TcshStartupArgs, generate_tcsh_startup_commands};
use crate::gen_rc::zsh::{ZshStartupArgs, generate_zsh_startup_commands};
use crate::gen_rc::{StartupArgs, StartupCtx};

pub const STARTUP_SCRIPT_PATH_OVERRIDE_VAR: &str = "_FLOX_RC_FILE_PATH";
pub const STARTUP_SCRIPT_NO_SELF_DESTRUCT_VAR: &str = "_FLOX_RC_FILE_NO_SELF_DESTRUCT";

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
    pub fn handle(self, subsystem_verbosity: u32) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.activate_data)?;
        let mut context: ActivateCtx = serde_json::from_str(&contents)?;

        if context.remove_after_reading {
            fs::remove_file(&self.activate_data)?;
        }

        // In the case of containerize, you can't bake-in the invocation type or the
        // `run_args`, so you need to do that detection at runtime. Here we do that
        // by modifying the `ActivateCtx` passed to us in the container's
        // EntryPoint.
        let run_args = self
            .cmd
            .as_ref()
            .or(Some(&context.run_args))
            .and_then(|args| if args.is_empty() { None } else { Some(args) });

        match (context.invocation_type.as_ref(), run_args) {
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, None) => context.invocation_type = Some(InvocationType::Interactive),
            // This is a container invocation, and we need to set the invocation type
            // based on the presence of command arguments.
            (None, Some(args)) => {
                context.invocation_type = Some(InvocationType::Command);
                context.run_args = args.clone();
            },
            // The following two cases are normal shell activations, and don't need
            // to modify the activation context.
            (Some(_), None) => {},
            (Some(_), Some(_)) => {},
        }
        // For any case where `invocation_type` is None, we should have detected that above
        // and set it to Some.
        let invocation_type = context
            .invocation_type
            .expect("invocation type should have been some");

        let start_or_attach = StartOrAttachArgs {
            pid: std::process::id() as i32,
            flox_env: PathBuf::from(&context.env),
            store_path: context.flox_activate_store_path.clone(),
            runtime_dir: PathBuf::from(&context.flox_runtime_dir),
        }
        .handle_inner()?;

        let vars_from_env = VarsFromEnvironment::get()?;

        if start_or_attach.attach {
            debug!(
                "Attaching to existing activation in state dir {:?}, id {}",
                start_or_attach.activation_state_dir, start_or_attach.activation_id
            );
            if invocation_type == InvocationType::Interactive {
                eprintln!(
                    "{}",
                    formatdoc! {"✅ Attached to existing activation of environment '{}'
                             To stop using this environment, type 'exit'
                            ",
                    context.env_description,
                    }
                );
            }
        } else {
            debug!("Starting activation");
            let mut start_command = assemble_command_for_start_script(
                context.clone(),
                subsystem_verbosity,
                vars_from_env.clone(),
                &start_or_attach,
                invocation_type,
            );
            start_command.spawn()?.wait()?;
        };

        let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;

        let _activate_tracer = activate_tracer(&context.interpreter_path);

        let activate_script_command = assemble_command_for_activate_script(
            "activate_temporary",
            context.clone(),
            subsystem_verbosity,
            vars_from_env,
            &diff,
            &start_or_attach,
        );
        // Create the path if we're going to need it (we won't for in-place).
        // We're doing this ahead of time here because it's shell-agnostic and the `match`
        // statement below is already going to be huge.
        let mut _rc_path = None;
        if invocation_type != InvocationType::InPlace {
            let path = if let Ok(rc_path_str) = std::env::var(STARTUP_SCRIPT_PATH_OVERRIDE_VAR) {
                PathBuf::from(rc_path_str)
            } else {
                let prefix = format!("flox_rc_{}_", context.shell.name());
                let tmp = tempfile::NamedTempFile::with_prefix_in(
                    prefix,
                    &start_or_attach.activation_state_dir,
                )?;
                let rc_path = tmp.path().to_path_buf();
                tmp.keep()?;
                rc_path
            };
            _rc_path = Some(path);
        }

        // NOTE: use Self::write_to_path or Self::write_to_stdout
        //  to actually write the script

        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        if invocation_type == InvocationType::InPlace {
            Self::activate_in_place(activate_script_command, context)?;

            return Ok(());
        }

        // These functions will only return if exec fails
        if invocation_type == InvocationType::Interactive {
            Self::activate_interactive(activate_script_command)
        } else {
            Self::activate_command(activate_script_command, context.run_args)
        }
    }

    #[allow(unused)]
    fn startup_ctx(
        ctx: ActivateCtx,
        invocation_type: InvocationType,
        rc_path: Option<PathBuf>,
        env_diff: EnvDiff,
        state_dir: &Path,
        activate_tracer: &str,
        subsystem_verbosity: u32,
    ) -> Result<StartupCtx> {
        let is_sourcing_rc = std::env::var("_flox_sourcing_rc").is_ok_and(|val| val == "true");
        let flox_activations = (*FLOX_ACTIVATIONS_BIN).clone();
        let self_destruct =
            !std::env::var(STARTUP_SCRIPT_NO_SELF_DESTRUCT_VAR).is_ok_and(|val| val == "true");

        let clean_up = if rc_path.is_some() && self_destruct {
            rc_path.clone()
        } else {
            None
        };

        let s_ctx = match ctx.shell {
            ShellWithPath::Bash(_) => {
                let bashrc_path = if let Some(home_dir) = dirs::home_dir() {
                    let bashrc_path = home_dir.join(".bashrc");
                    if bashrc_path.exists() {
                        Some(bashrc_path)
                    } else {
                        None
                    }
                } else {
                    return Err(anyhow!("failed to get home directory"));
                };
                let startup_args = BashStartupArgs {
                    flox_activate_tracelevel: subsystem_verbosity,
                    activate_d: ctx.interpreter_path.join("activate.d"),
                    flox_env: PathBuf::from(ctx.env.clone()),
                    flox_env_cache: Some(ctx.env_cache.clone()),
                    flox_env_project: ctx.env_project.clone(),
                    flox_env_description: Some(ctx.env_description.clone()),
                    is_in_place: invocation_type == InvocationType::InPlace,
                    bashrc_path,
                    flox_sourcing_rc: is_sourcing_rc,
                    flox_activate_tracer: activate_tracer.to_string(),
                    flox_activations,
                    clean_up,
                };
                StartupCtx {
                    args: StartupArgs::Bash(startup_args),
                    state_dir: state_dir.to_path_buf(),
                    env_diff,
                    rc_path,
                    act_ctx: ctx,
                }
            },
            ShellWithPath::Fish(_) => {
                let startup_args = FishStartupArgs {
                    flox_activate_tracelevel: subsystem_verbosity,
                    activate_d: ctx.interpreter_path.join("activate.d"),
                    flox_env: PathBuf::from(ctx.env.clone()),
                    flox_env_cache: Some(ctx.env_cache.clone()),
                    flox_env_project: ctx.env_project.clone(),
                    flox_env_description: Some(ctx.env_description.clone()),
                    is_in_place: invocation_type == InvocationType::InPlace,
                    flox_sourcing_rc: is_sourcing_rc,
                    flox_activate_tracer: activate_tracer.to_string(),
                    flox_activations,
                    clean_up,
                };
                StartupCtx {
                    args: StartupArgs::Fish(startup_args),
                    state_dir: state_dir.to_path_buf(),
                    env_diff,
                    rc_path,
                    act_ctx: ctx,
                }
            },
            ShellWithPath::Tcsh(_) => {
                let startup_args = TcshStartupArgs {
                    flox_activate_tracelevel: subsystem_verbosity,
                    activate_d: ctx.interpreter_path.join("activate.d"),
                    flox_env: PathBuf::from(ctx.env.clone()),
                    flox_env_cache: Some(ctx.env_cache.clone()),
                    flox_env_project: ctx.env_project.clone(),
                    flox_env_description: Some(ctx.env_description.clone()),
                    is_in_place: invocation_type == InvocationType::InPlace,
                    flox_sourcing_rc: is_sourcing_rc,
                    flox_activate_tracer: activate_tracer.to_string(),
                    flox_activations,
                    clean_up,
                };
                StartupCtx {
                    args: StartupArgs::Tcsh(startup_args),
                    state_dir: state_dir.to_path_buf(),
                    env_diff,
                    rc_path,
                    act_ctx: ctx,
                }
            },
            ShellWithPath::Zsh(_) => {
                let startup_args = ZshStartupArgs {
                    flox_activate_tracelevel: subsystem_verbosity,
                    activate_d: ctx.interpreter_path.join("activate.d"),
                    flox_env: PathBuf::from(ctx.env.clone()),
                    flox_env_cache: Some(ctx.env_cache.clone()),
                    flox_env_project: ctx.env_project.clone(),
                    flox_env_description: Some(ctx.env_description.clone()),
                    clean_up,
                    activation_state_dir: state_dir.to_path_buf(),
                };
                StartupCtx {
                    args: StartupArgs::Zsh(startup_args),
                    state_dir: state_dir.to_path_buf(),
                    env_diff,
                    rc_path,
                    act_ctx: ctx,
                }
            },
        };
        Ok(s_ctx)
    }

    #[allow(dead_code)]
    fn write_to_path(ctx: &StartupCtx, path: &Path) -> Result<()> {
        let mut writer = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(true)
            .open(path)?;
        match ctx.args {
            StartupArgs::Bash(ref args) => {
                generate_bash_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Fish(ref args) => {
                generate_fish_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Tcsh(ref args) => {
                generate_tcsh_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Zsh(ref args) => generate_zsh_startup_commands(args, &mut writer)?,
        }
        Ok(())
    }

    #[allow(dead_code)]
    fn write_to_stdout(ctx: &StartupCtx) -> Result<()> {
        let mut writer = std::io::stdout();
        match ctx.args {
            StartupArgs::Bash(ref args) => {
                generate_bash_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Fish(ref args) => {
                generate_fish_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Tcsh(ref args) => {
                generate_tcsh_startup_commands(args, &ctx.env_diff, &mut writer)?
            },
            StartupArgs::Zsh(ref args) => generate_zsh_startup_commands(args, &mut writer)?,
        }
        Ok(())
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
    fn activate_in_place(mut activate_script_command: Command, context: ActivateCtx) -> Result<()> {
        debug!("running activation command: {:?}", activate_script_command);

        let output = activate_script_command
            .output()
            .context("failed to run activation script")?;
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        let legacy_exports = Self::render_legacy_exports(context);

        let script = formatdoc! {"
            {legacy_exports}
            {output}
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");

        Ok(())
    }

    /// The CLI used to print export statements for in-place activations for
    /// every environment variable set prior to invoking the activate script
    fn render_legacy_exports(context: ActivateCtx) -> String {
        // Render the exports in the correct shell dialect.
        old_cli_envs(context.clone()).iter()
        .map(|(key, value)| {
            (key, shell_escape::escape(Cow::Borrowed(value)))
            })
            // TODO: we should use a method on Shell here, possibly using
            // shell_escape in the Shell method?
            // But not quoting here is intentional because we already use shell_escape
            .map(|(key, value)| match context.shell {
                ShellWithPath::Bash(_) => format!("export {key}={value};",),
                ShellWithPath::Fish(_) => format!("set -gx {key} {value};",),
                ShellWithPath::Tcsh(_) => format!("setenv {key} {value};",),
                ShellWithPath::Zsh(_) => format!("export {key}={value};",),
            })
            .join("\n")
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

#[derive(Clone, Debug)]
struct VarsFromEnvironment {
    flox_env_dirs: Option<String>,
    path: String,
    manpath: Option<String>,
}

impl VarsFromEnvironment {
    fn get() -> Result<Self> {
        let flox_env_dirs = std::env::var(FLOX_ENV_DIRS_VAR).ok();
        let path = match std::env::var("PATH") {
            Ok(path) => path,
            Err(e) => {
                return Err(anyhow!("failed to get PATH from environment: {}", e));
            },
        };
        let manpath = std::env::var("MANPATH").ok();

        Ok(Self {
            flox_env_dirs,
            path,
            manpath,
        })
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
