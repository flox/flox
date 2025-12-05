use std::borrow::Cow;
use std::fs::OpenOptions;
use std::io::{ErrorKind, IsTerminal};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use indoc::formatdoc;
use itertools::Itertools;
use nix::unistd::{close, dup2_stdin, pipe, write};
use shell_gen::{Shell, ShellWithPath};
use tracing::debug;

use crate::activate_script_builder::{activate_tracer, apply_activation_env, old_cli_envs};
use crate::cli::activate::{NO_REMOVE_ACTIVATION_FILES, VarsFromEnvironment};
use crate::cli::attach::{AttachArgs, AttachExclusiveArgs};
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;
use crate::gen_rc::bash::{BashStartupArgs, generate_bash_startup_commands};
use crate::gen_rc::fish::{FishStartupArgs, generate_fish_startup_commands};
use crate::gen_rc::tcsh::{TcshStartupArgs, generate_tcsh_startup_commands};
use crate::gen_rc::zsh::{ZshStartupArgs, generate_zsh_startup_commands};
use crate::gen_rc::{StartupArgs, StartupCtx};

pub const STARTUP_SCRIPT_PATH_OVERRIDE_VAR: &str = "_FLOX_RC_FILE_PATH";

pub fn attach(
    context: ActivateCtx,
    invocation_type: InvocationType,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach: StartOrAttachResult,
) -> Result<(), anyhow::Error> {
    let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;

    // Create the path if we're going to need it (we won't for in-place).
    // We're doing this ahead of time here because it's shell-agnostic and the `match`
    // statement below is already going to be huge.
    let mut rc_path = None;
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
        rc_path = Some(path);
    }
    let startup_ctx = startup_ctx(
        context.clone(),
        invocation_type.clone(),
        rc_path,
        diff.clone(),
        &start_or_attach.activation_state_dir,
        &activate_tracer(&context.interpreter_path),
        subsystem_verbosity,
    )?;

    match invocation_type {
        // when output is not a tty, and no command is provided
        // we just print an activation script to stdout
        //
        // That script can then be `eval`ed in the current shell,
        // e.g. in a .bashrc or .zshrc file:
        //
        //    eval "$(flox activate)"
        InvocationType::InPlace => {
            activate_in_place(startup_ctx, start_or_attach.activation_id)?;
            Ok(())
        },
        // All other invocation types only return if exec fails
        InvocationType::Interactive => activate_interactive(
            startup_ctx,
            subsystem_verbosity,
            vars_from_env,
            &start_or_attach,
        ),
        InvocationType::ShellCommand(shell_command) => activate_shell_command(
            shell_command,
            startup_ctx,
            subsystem_verbosity,
            vars_from_env,
            &start_or_attach,
        ),
        InvocationType::ExecCommand(exec_command) => activate_exec_command(
            exec_command,
            startup_ctx,
            subsystem_verbosity,
            vars_from_env,
            &start_or_attach,
        ),
    }
}

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
    let self_destruct = !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true");

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

/// Used for `flox activate -- exec_command ...`
fn activate_exec_command(
    exec_command: Vec<String>,
    startup_ctx: StartupCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach_result: &StartOrAttachResult,
) -> Result<()> {
    if exec_command.is_empty() {
        return Err(anyhow!("empty command provided"));
    }
    let mut command = Command::new(&exec_command[0]);
    if exec_command.len() > 1 {
        command.args(&exec_command[1..]);
    };

    apply_activation_env(
        &mut command,
        startup_ctx.act_ctx.clone(),
        subsystem_verbosity,
        vars_from_env,
        &startup_ctx.env_diff,
        start_or_attach_result,
    );

    debug!("executing command directly: {:?}", command);

    // exec replaces the current process - only returns on error
    let err = command.exec();
    match err.kind() {
        ErrorKind::NotFound => Err(anyhow!("{}: command not found", exec_command[0])),
        _ => Err(err.into()),
    }
}

/// Used for `flox activate -c shell_command`
fn activate_shell_command(
    shell_command: String,
    startup_ctx: StartupCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach_result: &StartOrAttachResult,
) -> Result<()> {
    let mut command = Command::new(startup_ctx.act_ctx.shell.exe_path());
    apply_activation_env(
        &mut command,
        startup_ctx.act_ctx.clone(),
        subsystem_verbosity,
        vars_from_env,
        &startup_ctx.env_diff,
        start_or_attach_result,
    );

    let rcfile = startup_ctx
        .rc_path
        .clone()
        .expect("rc_path should be some for command invocation");
    write_to_path(&startup_ctx, &rcfile)?;
    let rcfile = rcfile.to_string_lossy();

    match startup_ctx.act_ctx.shell {
        ShellWithPath::Bash(_) => {
            // TODO: I think we need to be checking standard input and error, not stdout
            // Per man bash:
            // An interactive shell is one...whose standard input and error are both connected to terminals (as determined by isatty(3))
            //
            // But I preserved the behavior on main.
            // Running from main, profile scripts aren't run unless stdout is a pipe
            // > flox list -c
            // version = 1
            //
            // [profile]
            // bash = '''
            //   echo hello profile.bash
            // '''
            // > FLOX_SHELL=bash flox activate -- true
            // > FLOX_SHELL=bash flox activate -- true | cat
            // hello profile.bash
            if std::io::stdout().is_terminal() {
                command.args(["--noprofile", "--rcfile", &rcfile, "-c", &shell_command]);
            } else {
                // Non-interactive: source via stdin
                // The bash --rcfile option only works for interactive shells
                // so we need to cobble together our own means of sourcing our
                // startup script for non-interactive shells.
                // Equivalent to: exec bash --noprofile --norc -s <<< "source '$RCFILE' && $*"

                command.arg("--noprofile").arg("--norc").arg("-s");

                let source_script = format!("source '{}' && {shell_command}\n", rcfile);

                // - create a pipe
                // - dup2 the read end to stdin, so that after exec'ing reading from stdin reads from the pipe
                // - close the read end of the pipe since it's now dup2'd to stdin
                // - write the source line to the write end of the pipe
                // - close the write end of the pipe since we've written all we need to
                let (read_fd, write_fd) = pipe()?;

                dup2_stdin(&read_fd)?;
                close(read_fd)?;

                write(&write_fd, source_script.as_bytes())?;
                close(write_fd)?;
            }
        },
        ShellWithPath::Fish(_) => {
            command.args([
                "--init-command",
                &format!("source '{}'", rcfile),
                "-c",
                &shell_command,
            ]);
        },
        ShellWithPath::Tcsh(_) => {
            // The tcsh implementation will source our custom `~/.tcshrc`,
            // which eventually sources $FLOX_TCSH_INIT_SCRIPT after the normal initialization.
            let home = std::env::var("HOME").unwrap_or("".to_string());
            command.env("FLOX_ORIG_HOME", home);
            let tcsh_home = startup_ctx
                .act_ctx
                .interpreter_path
                .join("activate.d/tcsh_home");
            command.env("HOME", tcsh_home.to_string_lossy().to_string());
            command.env("FLOX_TCSH_INIT_SCRIPT", &*rcfile);

            // The -m option is required for tcsh to source a .tcshrc file that
            // the effective user does not own.
            command.args(["-m", "-c", &shell_command]);
        },
        ShellWithPath::Zsh(_) => {
            // Save original ZDOTDIR if it exists
            if let Ok(zdotdir) = std::env::var("ZDOTDIR")
                && !zdotdir.is_empty()
            {
                command.env("FLOX_ORIG_ZDOTDIR", zdotdir);
            }
            let zdotdir = startup_ctx
                .act_ctx
                .interpreter_path
                .join("activate.d/zdotdir");
            command.env("ZDOTDIR", zdotdir.to_string_lossy().to_string());
            command.env("FLOX_ZSH_INIT_SCRIPT", &*rcfile);

            // The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
            // automatically sourcing /etc/zshrc et al.
            command.args(["-o", "NO_GLOBAL_RCS", "-c", &shell_command]);
        },
    }

    debug!("running activation command: {:?}", command);

    // exec should never return
    Err(command.exec().into())
}

/// Activate the environment interactively by spawning a new shell
/// and running the respective activation scripts.
///
/// This function should never return as it replaces the current process
fn activate_interactive(
    startup_ctx: StartupCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach_result: &StartOrAttachResult,
) -> Result<()> {
    let mut command = Command::new(startup_ctx.act_ctx.shell.exe_path());
    apply_activation_env(
        &mut command,
        startup_ctx.act_ctx.clone(),
        subsystem_verbosity,
        vars_from_env,
        &startup_ctx.env_diff,
        start_or_attach_result,
    );

    let rcfile = startup_ctx
        .rc_path
        .clone()
        .expect("rc_path should be some for interactive invocation");
    write_to_path(&startup_ctx, &rcfile)?;
    let rcfile = rcfile.to_string_lossy();

    match startup_ctx.act_ctx.shell {
        ShellWithPath::Bash(_) => {
            if std::io::stdout().is_terminal() {
                command.args(["--noprofile", "--rcfile", &rcfile]);
            } else {
                // Non-interactive: source via stdin
                // Equivalent to: exec bash --noprofile --norc <<< "source '$RCFILE'"
                // The bash --rcfile option only works for interactive shells
                // so we need to cobble together our own means of sourcing our
                // startup script for non-interactive shells.
                // XXX Is this case even a thing? What's the point of activating with
                //     no command to be invoked and no controlling terminal from which
                //     to issue commands?!? A broken docker experience maybe?!?
                command.arg("--noprofile").arg("--norc").arg("-s");

                let source_script = format!("source '{}'\n", rcfile);

                // - create a pipe
                // - dup2 the read end to stdin, so that after exec'ing reading from stdin reads from the pipe
                // - close the read end of the pipe since it's now dup2'd to stdin
                // - write the source line to the write end of the pipe
                // - close the write end of the pipe since we've written all we need to
                let (read_fd, write_fd) = pipe()?;

                dup2_stdin(&read_fd)?;
                close(read_fd)?;

                write(&write_fd, source_script.as_bytes())?;
                close(write_fd)?;
            }
        },
        ShellWithPath::Fish(_) => {
            command.args(["--init-command", &format!("source '{}'", rcfile)]);
        },
        ShellWithPath::Tcsh(_) => {
            // The tcsh implementation will source our custom `~/.tcshrc`,
            // which eventually sources $FLOX_TCSH_INIT_SCRIPT after the normal initialization.
            let home = std::env::var("HOME").unwrap_or("".to_string());
            command.env("FLOX_ORIG_HOME", home);
            let tcsh_home = startup_ctx
                .act_ctx
                .interpreter_path
                .join("activate.d/tcsh_home");
            command.env("HOME", tcsh_home.to_string_lossy().to_string());
            command.env("FLOX_TCSH_INIT_SCRIPT", &*rcfile);

            // The -m option is required for tcsh to source a .tcshrc file that
            // the effective user does not own.
            command.args(["-m"]);
        },
        ShellWithPath::Zsh(_) => {
            // Save original ZDOTDIR if it exists
            if let Ok(zdotdir) = std::env::var("ZDOTDIR")
                && !zdotdir.is_empty()
            {
                command.env("FLOX_ORIG_ZDOTDIR", zdotdir);
            }
            let zdotdir = startup_ctx
                .act_ctx
                .interpreter_path
                .join("activate.d/zdotdir");
            command.env("ZDOTDIR", zdotdir.to_string_lossy().to_string());
            command.env("FLOX_ZSH_INIT_SCRIPT", &*rcfile);

            // The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
            // automatically sourcing /etc/zshrc et al.
            command.args(["-o", "NO_GLOBAL_RCS"]);
        },
    }

    debug!("running activation command: {:?}", command);

    // exec should never return
    Err(command.exec().into())
}

/// Used for `eval "$(flox activate)"`
fn activate_in_place(startup_ctx: StartupCtx, activation_id: String) -> Result<()> {
    let attach_command = AttachArgs {
        pid: std::process::id() as i32,
        flox_env: (&startup_ctx.act_ctx.env).into(),
        id: activation_id.clone(),
        exclusive: AttachExclusiveArgs {
            timeout_ms: Some(5000),
            remove_pid: None,
        },
        runtime_dir: (&startup_ctx.act_ctx.flox_runtime_dir).into(),
    };

    // Put a 5 second timeout on the activation
    attach_command.handle()?;

    let legacy_exports = render_legacy_exports(startup_ctx.act_ctx.clone());

    let exports_for_zsh = if matches!(startup_ctx.act_ctx.shell, ShellWithPath::Zsh(_)) {
        let zdotdir_path = startup_ctx
            .act_ctx
            .interpreter_path
            .join("activate.d/zdotdir");
        let mut exports = String::new();

        // TODO: it would probably be better to just not touch ZDOTDIR in
        // the zsh startup script if invocation type is in-place
        if let Ok(current_zdotdir) = std::env::var("ZDOTDIR")
            && !current_zdotdir.is_empty()
        {
            exports.push_str(&format!(
                "export FLOX_ORIG_ZDOTDIR=\"{}\";\n",
                current_zdotdir
            ));
        }
        exports.push_str(&format!("export ZDOTDIR=\"{}\";\n", zdotdir_path.display()));

        exports.push_str(&format!(
            "export _flox_activate_tracer=\"{}\";\n",
            activate_tracer(&startup_ctx.act_ctx.interpreter_path)
        ));

        exports
    } else {
        String::new()
    };

    let script = formatdoc! {r#"
            {legacy_exports}
            {flox_activations} attach --runtime-dir "{runtime_dir}" --pid {self_pid_var} --flox-env "{flox_env}" --id "{id}" --remove-pid "{pid}";
            {exports_for_zsh}
        "#,
        flox_activations = (*FLOX_ACTIVATIONS_BIN).to_string_lossy(),
        runtime_dir = startup_ctx.act_ctx.flox_runtime_dir,
        self_pid_var = Shell::from(startup_ctx.act_ctx.shell.clone()).self_pid_var(),
        flox_env = startup_ctx.act_ctx.env,
        id = activation_id,
        pid = std::process::id(),
    };

    print!("{script}");
    debug!(
        "activation in place script, except for startup commands:\n{}",
        script
    );
    write_to_stdout(&startup_ctx)?;

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
pub fn quote_run_args(run_args: &[String]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quote_run_args() {
        assert_eq!(
            quote_run_args(&["a b".to_string(), '"'.to_string()]),
            r#""a b" "\"""#
        )
    }
}
