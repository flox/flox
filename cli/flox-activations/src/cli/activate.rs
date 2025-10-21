use std::collections::HashMap;
use std::env::Vars;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
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
use super::attach::AttachArgs;
use super::fix_paths::{fix_manpath_var, fix_path_var};
use super::set_env_dirs::fix_env_dirs_var;
use super::start_or_attach::wait_for_activation_ready_and_optionally_attach_pid;
use crate::cli::attach::AttachExclusiveArgs;
use crate::shell_gen::bash::{BashStartupArgs, generate_bash_startup_commands};
use crate::shell_gen::capture::{EnvDiff, ExportEnvDiff};

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

// TODO
pub const RM: &str = "rm";

impl ActivateArgs {
    pub fn handle(self, verbosity: u8) -> Result<(), anyhow::Error> {
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
            debug!("No existing activation found");
            let mut start_command = Self::assemble_command_for_activate_script(data.clone());
            start_command.arg("--mode").arg("start");
            start_command
                .arg("--activation-state-dir")
                .arg(activation_state_dir.to_string_lossy().to_string());
            start_command.arg("--activation-id").arg(&activation_id);
            start_command
                // Hooks may use stdin, stdout, stderr
                .stderr(Stdio::inherit())
                .stdout(Stdio::inherit())
                .stdin(Stdio::inherit())
                .spawn()?;

            let attach_expiration = OffsetDateTime::now_utc() + Duration::seconds(10);
            wait_for_activation_ready_and_optionally_attach_pid(
                &activations_json_path(&data.flox_runtime_dir, data.env.clone()),
                &data.flox_activate_store_path,
                attach_expiration,
                None,
            )?;
        }
        let mut command = Self::assemble_command_for_activate_script(data.clone());
        // Pass down the activation mode
        command.arg("--mode").arg(&data.mode);
        command
            .arg("--activation-state-dir")
            .arg(activation_state_dir.to_string_lossy().to_string());
        command.arg("--activation-id").arg(&activation_id);

        if matches!(data.shell, Shell::Bash(_)) {
            let export_env_diff = ExportEnvDiff::from_files(
                activation_state_dir.join("add.env"),
                activation_state_dir.join("del.env"),
            )?;
            let env_diff: EnvDiff = (&export_env_diff).try_into()?;
            let vars_from_environment = VarsFromEnvironment::get()?;
            let activation_environment =
                Self::assemble_environment(data.clone(), vars_from_environment, env_diff)?;
            if attach {
                // TODO: print message about attaching
            }
            if data.flox_activate_start_services {
                Self::start_services();
            }

            if data.in_place {
                let flox_sourcing_rc = std::env::var("_flox_sourcing_rc")
                    .map(|v| v == "true")
                    .unwrap_or(false);
                let legacy_exports = Self::render_legacy_exports(&command, &data.shell);
                Self::new_activate_in_place(
                    data,
                    activation_id,
                    activation_state_dir,
                    legacy_exports,
                    flox_sourcing_rc,
                    verbosity,
                    export_env_diff,
                )?;
                return Ok(());
            }

            // These functions will only return if exec fails or for an
            // ephemeral activation
            if data.interactive {
                Self::new_activate_interactive(
                    verbosity,
                    data,
                    std::env::var("_flox_sourcing_rc")
                        .map(|v| v == "true")
                        .unwrap_or(false),
                    export_env_diff,
                    &activation_state_dir,
                )?;
            } else {
                Self::new_activate_command(
                    data.run_args,
                    data.is_ephemeral,
                    activation_environment,
                )?;
            }

            return Ok(());
        }

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

    fn assemble_command_for_activate_script(data: ActivateData) -> Command {
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

    fn assemble_command_with_environment(
        bin: impl AsRef<OsStr>,
        activate_environment: &EnvDiff,
    ) -> Command {
        let mut command = Command::new(bin);

        command.envs(&activate_environment.additions);
        for key in &activate_environment.deletions {
            command.env_remove(key);
        }

        command
    }

    fn assemble_environment(
        data: ActivateData,
        vars_from_environment: VarsFromEnvironment,
        mut env_diff: EnvDiff,
    ) -> Result<EnvDiff> {
        let mut additions_static_str = HashMap::new();

        additions_static_str.extend(Self::assemble_fixed_vars(&data.env, vars_from_environment));

        // TODO: dedup with shell_gen specific code
        // Propagate required variables that are documented as exposed.
        additions_static_str.insert("FLOX_ENV", data.env);
        // Propagate optional variables that are documented as exposed.
        additions_static_str.insert(
            "FLOX_ENV_CACHE",
            data.env_cache.to_string_lossy().to_string(),
        );

        additions_static_str.insert(
            "FLOX_ENV_PROJECT",
            data.env_project.to_string_lossy().to_string(),
        );

        additions_static_str.insert("FLOX_ENV_DESCRIPTION", data.env_description);

        // Do we need this or will this already be inherited?
        additions_static_str.extend(default_nix_env_vars());

        env_diff.additions.extend(
            additions_static_str
                .into_iter()
                .map(|(k, v)| (k.to_string(), v)),
        );

        Ok(env_diff)
    }

    fn assemble_fixed_vars(
        flox_env: impl AsRef<str>,
        vars_from_environment: VarsFromEnvironment,
    ) -> HashMap<&'static str, String> {
        let new_flox_env_dirs = fix_env_dirs_var(
            flox_env.as_ref(),
            vars_from_environment
                .flox_env_dirs
                .unwrap_or("".to_string()),
        );
        let new_path = fix_path_var(&new_flox_env_dirs, &vars_from_environment.path);
        let new_manpath = fix_manpath_var(
            &new_flox_env_dirs,
            &vars_from_environment.manpath.unwrap_or("".to_string()),
        );
        HashMap::from([
            ("FLOX_ENV_DIRS", new_flox_env_dirs),
            ("PATH", new_path),
            ("MANPATH", new_manpath),
        ])
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

    fn new_activate_command(
        run_args: Vec<String>,
        is_ephemeral: bool,
        env_diff: EnvDiff,
    ) -> Result<()> {
        if run_args.is_empty() {
            return Err(anyhow!("empty command provided"));
        }
        let user_command = &run_args[0];
        let args = &run_args[1..];

        let mut command = Self::assemble_command_with_environment(user_command, &env_diff);
        command.args(args);
        if is_ephemeral {
            let output = command
                .stderr(Stdio::piped())
                .stdout(Stdio::piped())
                .output()?;
            if !output.status.success() {
                Err(anyhow!(
                    "failed to run command: {}",
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

    fn new_activate_interactive(
        verbosity: u8,
        data: ActivateData,
        flox_sourcing_rc: bool,
        export_env_diff: ExportEnvDiff,
        activation_state_dir: &PathBuf,
    ) -> Result<()> {
        match data.shell {
            Shell::Bash(bash) => {
                let bash_startup_args = BashStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                };
                let startup_commands =
                    generate_bash_startup_commands(&bash_startup_args, &export_env_diff)?;
                let rcfile = Self::write_maybe_self_destructing_script(
                    startup_commands,
                    activation_state_dir,
                    verbosity < 2,
                )?;
                let mut command = Command::new(bash);
                command.args(["--noprofile", "--rcfile", &rcfile.to_string_lossy()]);

                debug!("spawning interactive bash shell: {:?}", command);
                // exec should never return
                Err(command.exec().into())
                // TODO: do we need to port this case?
                // # The bash --rcfile option only works for interactive shells
                // # so we need to cobble together our own means of sourcing our
                // # startup script for non-interactive shells.
                // # XXX Is this case even a thing? What's the point of activating with
                // #     no command to be invoked and no controlling terminal from which
                // #     to issue commands?!? A broken docker experience maybe?!?
                // exec "$_flox_shell" --noprofile --norc -s <<< "source '$RCFILE'"
            },
            _ => unimplemented!(),
        }
    }

    fn write_maybe_self_destructing_script(
        mut script: String,
        activation_state_dir: &PathBuf,
        self_destruct: bool,
    ) -> Result<PathBuf> {
        let mut tempfile = tempfile::NamedTempFile::new_in(activation_state_dir)?;
        if self_destruct {
            script.push_str(&format!("\n{RM} {}", tempfile.path().to_string_lossy()));
        }
        tempfile.write_all(script.as_bytes())?;
        let (_, path) = tempfile.keep()?;
        Ok(path)
    }

    fn render_legacy_exports(command: &Command, shell: &Shell) -> String {
        // Render the exports in the correct shell dialect.
        command
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
            .join("\n")
    }

    /// Used for `eval "$(flox activate)"`
    fn activate_in_place(mut command: Command, shell: Shell) -> Result<()> {
        debug!("running activation command: {:?}", command);

        let output = command
            .output()
            .context("failed to run activation script")?;
        eprint!("{}", String::from_utf8_lossy(&output.stderr));

        let exports_rendered = Self::render_legacy_exports(&command, &shell);

        let script = formatdoc! {"
            {exports_rendered}
            {output}
        ",
        output = String::from_utf8_lossy(&output.stdout),
        };

        print!("{script}");

        Ok(())
    }

    fn new_activate_in_place(
        data: ActivateData,
        activation_id: String,
        activation_state_dir: PathBuf,
        legacy_exports: String,
        flox_sourcing_rc: bool,
        verbosity: u8,
        export_env_diff: ExportEnvDiff,
    ) -> Result<()> {
        let attach_command = AttachArgs {
            pid: std::process::id() as i32,
            flox_env: (&data.env).into(),
            id: activation_id.clone(),
            exclusive: AttachExclusiveArgs {
                timeout_ms: Some(5000),
                remove_pid: None,
            },
            runtime_dir: (&data.flox_runtime_dir).into(),
        };
        // Put a 5 second timeout on the activation
        attach_command.handle()?;
        let startup_commands = match data.shell {
            Shell::Bash(_) => {
                let bash_startup_args = BashStartupArgs {
                    flox_activate_tracelevel: verbosity as i32,
                    activate_d: data.interpreter_path.join("activate.d"),
                    flox_env: data.env.clone(),
                    flox_env_cache: Some(data.env_cache.to_string_lossy().to_string()),
                    flox_env_project: Some(data.env_project.to_string_lossy().to_string()),
                    flox_env_description: Some(data.env_description),
                    is_in_place: data.in_place,
                    flox_sourcing_rc,
                    flox_activations: (&data.path_to_self).into(),
                };
                let startup_commands =
                    generate_bash_startup_commands(&bash_startup_args, &export_env_diff)?;

                formatdoc! {r#"
                  {flox_activations} attach --runtime-dir "{runtime_dir}" --pid $$ --flox-env "{flox_env}" --id {id} --remove-pid {pid};
                  {startup_commands}
                "#,
                // TODO: this should probably be based on interpreter_path
                flox_activations = data.path_to_self,
                runtime_dir = data.flox_runtime_dir,
                flox_env = data.env,
                id = activation_id,
                pid = std::process::id() }
            },
            _ => unimplemented!(),
        };
        let script = formatdoc! {"
            {legacy_exports}
            {startup_commands}
        "};
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

    fn start_services() {
        todo!();
    }
}

struct VarsFromEnvironment {
    flox_env_dirs: Option<String>,
    path: String,
    manpath: Option<String>,
}

impl VarsFromEnvironment {
    fn get() -> Result<Self> {
        let flox_env_dirs = std::env::var("FLOX_ENV_DIRS").ok();
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
