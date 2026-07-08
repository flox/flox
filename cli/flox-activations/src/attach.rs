use std::fs::OpenOptions;
use std::io::{ErrorKind, IsTerminal};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, anyhow};
use flox_core::activate::context::{ActivateCtx, InvocationType};
use flox_core::activate::vars::FLOX_ACTIVATIONS_BIN;
use flox_core::activations::StartIdentifier;
use indoc::formatdoc;
use nix::unistd::{close, dup2_stdin, pipe, write};
use shell_gen::{Shell, ShellWithPath};
use tracing::debug;

use crate::attach_diff::{AttachDiff, activate_tracer};
use crate::cli::activate::NO_REMOVE_ACTIVATION_FILES;
use crate::cli::attach::{AttachArgs, AttachExclusiveArgs};
use crate::gen_rc::bash::{BashStartupArgs, generate_bash_profile_commands};
use crate::gen_rc::fish::{FishStartupArgs, generate_fish_profile_commands};
use crate::gen_rc::tcsh::{TcshStartupArgs, generate_tcsh_profile_commands};
use crate::gen_rc::zsh::{ZshStartupArgs, generate_zsh_profile_commands};
use crate::gen_rc::{Action, ShellStartupArgs, StartupCtx};
use crate::start_diff::StartDiff;
use crate::vars_from_env::VarsFromEnvironment;

pub const STARTUP_SCRIPT_PATH_OVERRIDE_VAR: &str = "_FLOX_RC_FILE_PATH";

pub fn attach(
    context: ActivateCtx,
    invocation_type: InvocationType,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_id: StartIdentifier,
) -> Result<(), anyhow::Error> {
    // Use pre-computed activation_state_dir to get start state directory
    let start_state_dir = start_id.start_state_dir(&context.activation_state_dir)?;
    let diff = StartDiff::from_files(&start_state_dir)?;

    // Create the path if we're going to need it (we won't for in-place).
    // We're doing this ahead of time here because it's shell-agnostic and the `match`
    // statement below is already going to be huge.
    let mut rc_path = None;
    if invocation_type != InvocationType::InPlace {
        let path = if let Ok(rc_path_str) = std::env::var(STARTUP_SCRIPT_PATH_OVERRIDE_VAR) {
            PathBuf::from(rc_path_str)
        } else {
            let prefix = format!("flox_rc_{}_", context.shell.name());
            let tmp = tempfile::NamedTempFile::with_prefix_in(prefix, &start_state_dir)?;
            let rc_path = tmp.path().to_path_buf();
            tmp.keep()?;
            rc_path
        };
        rc_path = Some(path);
    }
    let tracer = activate_tracer(&context.attach_ctx.interpreter_path);

    let is_sourcing_rc = std::env::var("_flox_sourcing_rc").is_ok_and(|val| val == "true");
    let self_destruct = !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true");
    let bashrc_path = if matches!(context.shell, ShellWithPath::Bash(_)) {
        let home_dir = dirs::home_dir().ok_or_else(|| anyhow!("failed to get home directory"))?;
        let bashrc_path = home_dir.join(".bashrc");
        bashrc_path.exists().then_some(bashrc_path)
    } else {
        None
    };

    let startup_ctx = startup_ctx(
        context,
        invocation_type.clone(),
        rc_path,
        diff,
        &tracer,
        subsystem_verbosity,
        vars_from_env,
        is_sourcing_rc,
        self_destruct,
        bashrc_path,
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
            activate_in_place(startup_ctx, start_id)?;
            Ok(())
        },
        // All other invocation types only return if exec fails
        InvocationType::Interactive => activate_interactive(startup_ctx),
        InvocationType::ShellCommand(shell_command) => {
            activate_shell_command(shell_command, startup_ctx)
        },
        InvocationType::ExecCommand(exec_command) => {
            activate_exec_command(exec_command, startup_ctx)
        },
    }
}

/// Build startup context for shell configuration.
/// Used by both normal activations (with project context) and containers (without).
#[allow(clippy::too_many_arguments)]
pub(crate) fn startup_ctx(
    ctx: ActivateCtx,
    invocation_type: InvocationType,
    rc_path: Option<PathBuf>,
    start_diff: StartDiff,
    activate_tracer: &str,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    is_sourcing_rc: bool,
    self_destruct: bool,
    bashrc_path: Option<PathBuf>,
) -> Result<StartupCtx> {
    let flox_activations = (*FLOX_ACTIVATIONS_BIN).clone();

    let clean_up = if rc_path.is_some() && self_destruct {
        rc_path.clone()
    } else {
        None
    };

    let attach_diff = AttachDiff::new(
        &ctx.attach_ctx,
        ctx.project_ctx.as_ref(),
        subsystem_verbosity,
        vars_from_env,
        &start_diff,
        invocation_type.is_in_place(),
    )?;

    let set_prompt = ctx.attach_ctx.set_prompt;

    // Respect the disable_hook config option
    let register_hook = !ctx.disable_hook;

    // Container activations have no project context. Emit a `flox()` shim
    // in the rcfile so `flox deactivate` maps to `exit` even when the flox
    // binary is absent from the image. Bash-only: mkContainer.nix always
    // bakes `shell = bash`.
    let container_shim = ctx.project_ctx.is_none();

    let args = match ctx.shell {
        ShellWithPath::Bash(_) => ShellStartupArgs::Bash(BashStartupArgs {
            flox_activate_tracelevel: subsystem_verbosity,
            activate_d: ctx.attach_ctx.interpreter_path.join("activate.d"),
            flox_env: PathBuf::from(ctx.attach_ctx.env.clone()),
            invocation_type,
            bashrc_path,
            flox_sourcing_rc: is_sourcing_rc,
            flox_activate_tracer: activate_tracer.to_string(),
            flox_activations,
            clean_up,
            register_hook,
            flox_bin: ctx.flox_bin.clone(),
            set_prompt,
            container_shim,
        }),
        ShellWithPath::Fish(_) => ShellStartupArgs::Fish(FishStartupArgs {
            flox_activate_tracelevel: subsystem_verbosity,
            activate_d: ctx.attach_ctx.interpreter_path.join("activate.d"),
            flox_env: PathBuf::from(ctx.attach_ctx.env.clone()),
            invocation_type,
            flox_sourcing_rc: is_sourcing_rc,
            flox_activate_tracer: activate_tracer.to_string(),
            flox_activations,
            clean_up,
            register_hook,
            flox_bin: ctx.flox_bin.clone(),
            auto_activate_fish_mode: ctx.auto_activate_fish_mode,
            set_prompt,
        }),
        ShellWithPath::Tcsh(_) => ShellStartupArgs::Tcsh(TcshStartupArgs {
            flox_activate_tracelevel: subsystem_verbosity,
            activate_d: ctx.attach_ctx.interpreter_path.join("activate.d"),
            flox_env: PathBuf::from(ctx.attach_ctx.env.clone()),
            invocation_type,
            flox_sourcing_rc: is_sourcing_rc,
            flox_activate_tracer: activate_tracer.to_string(),
            flox_activations,
            clean_up,
            register_hook,
            flox_bin: ctx.flox_bin.clone(),
            set_prompt,
        }),
        ShellWithPath::Zsh(_) => ShellStartupArgs::Zsh(ZshStartupArgs {
            flox_activate_tracelevel: subsystem_verbosity,
            activate_d: ctx.attach_ctx.interpreter_path.join("activate.d"),
            invocation_type,
            clean_up,
            register_hook,
            flox_bin: ctx.flox_bin.clone(),
            set_prompt,
        }),
    };

    Ok(StartupCtx {
        args,
        rc_path,
        act_ctx: ctx,
        attach_diff,
    })
}

pub(crate) fn write_to_writer(ctx: &StartupCtx, writer: &mut impl std::io::Write) -> Result<()> {
    let attach_diff = ctx.attach_diff.clone();
    match &ctx.args {
        ShellStartupArgs::Bash(args) => {
            let action = Action::Activate {
                args: args.clone(),
                attach_diff,
            };
            generate_bash_profile_commands(&action, writer)?
        },
        ShellStartupArgs::Fish(args) => {
            let action = Action::Activate {
                args: args.clone(),
                attach_diff,
            };
            generate_fish_profile_commands(&action, writer)?
        },
        ShellStartupArgs::Tcsh(args) => {
            let action = Action::Activate {
                args: args.clone(),
                attach_diff,
            };
            generate_tcsh_profile_commands(&action, writer)?
        },
        ShellStartupArgs::Zsh(args) => {
            let action = Action::Activate {
                args: args.clone(),
                attach_diff,
            };
            generate_zsh_profile_commands(&action, writer)?
        },
    }
    Ok(())
}

fn write_to_path(ctx: &StartupCtx, path: &Path) -> Result<()> {
    let mut writer = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(true)
        .open(path)?;
    write_to_writer(ctx, &mut writer)
}

fn write_to_stdout(ctx: &StartupCtx) -> Result<()> {
    let mut writer = std::io::stdout();
    write_to_writer(ctx, &mut writer)
}

/// Used for `flox activate -- exec_command ...`
fn activate_exec_command(exec_command: Vec<String>, startup_ctx: StartupCtx) -> Result<()> {
    if exec_command.is_empty() {
        return Err(anyhow!("empty command provided"));
    }
    let mut command = Command::new(&exec_command[0]);
    if exec_command.len() > 1 {
        command.args(&exec_command[1..]);
    };
    startup_ctx.attach_diff.apply_to_command(&mut command);

    debug!("executing command directly: {:?}", command);

    // exec replaces the current process - only returns on error
    let err = command.exec();
    match err.kind() {
        ErrorKind::NotFound => Err(anyhow!("{}: command not found", exec_command[0])),
        _ => Err(err.into()),
    }
}

/// Used for `flox activate -c shell_command`
fn activate_shell_command(shell_command: String, startup_ctx: StartupCtx) -> Result<()> {
    let mut command = Command::new(startup_ctx.act_ctx.shell.exe_path());
    startup_ctx.attach_diff.apply_to_command(&mut command);

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
                .attach_ctx
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
                .attach_ctx
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
fn activate_interactive(startup_ctx: StartupCtx) -> Result<()> {
    let mut command = Command::new(startup_ctx.act_ctx.shell.exe_path());
    startup_ctx.attach_diff.apply_to_command(&mut command);

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
                .attach_ctx
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
                .attach_ctx
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
fn activate_in_place(startup_ctx: StartupCtx, start_id: StartIdentifier) -> Result<()> {
    let attach_command = AttachArgs {
        pid: std::process::id() as i32,
        activation_state_dir: startup_ctx.act_ctx.activation_state_dir.clone(),
        store_path: start_id.store_path.clone(),
        timestamp: start_id.timestamp.clone(),
        exclusive: AttachExclusiveArgs {
            timeout_ms: Some(5000),
            remove_pid: None,
        },
    };

    // Put a 5 second timeout on the activation
    attach_command.handle()?;

    let exports_for_zsh = if matches!(startup_ctx.act_ctx.shell, ShellWithPath::Zsh(_)) {
        let zdotdir_path = startup_ctx
            .act_ctx
            .attach_ctx
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
            activate_tracer(&startup_ctx.act_ctx.attach_ctx.interpreter_path)
        ));

        exports
    } else {
        String::new()
    };

    let script = formatdoc! {r#"
            {flox_activations} attach --activation-state-dir "{activation_state_dir}" --pid {self_pid_var} --store-path "{store_path}" --timestamp "{timestamp}" --remove-pid "{pid}";
            {exports_for_zsh}
        "#,
        flox_activations = (*FLOX_ACTIVATIONS_BIN).to_string_lossy(),
        activation_state_dir = startup_ctx.act_ctx.activation_state_dir.to_string_lossy(),
        self_pid_var = Shell::from(startup_ctx.act_ctx.shell.clone()).self_pid_var(),
        store_path = start_id.store_path.to_string_lossy(),
        timestamp = start_id.timestamp,
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
