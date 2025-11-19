use std::fs::{self};
use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use indoc::formatdoc;
use log::debug;

use super::StartOrAttachArgs;
use crate::activate_script_builder::{
    FLOX_ENV_DIRS_VAR,
    assemble_command_for_activate_script,
    assemble_command_for_start_script,
};
use crate::attach::attach;
use crate::env_diff::EnvDiff;

pub const NO_REMOVE_ACTIVATION_FILES: &str = "_FLOX_NO_REMOVE_ACTIVATION_FILES";

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

        if context.remove_after_reading
            && !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true")
        {
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

        if let Ok(shell_force) = std::env::var("_FLOX_SHELL_FORCE") {
            context.shell = PathBuf::from(shell_force).as_path().try_into()?;
        }
        // Unset FLOX_SHELL to detect the parent shell anew with each flox invocation.
        unsafe { std::env::remove_var("FLOX_SHELL") };

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
        }

        if context.flox_activate_start_services {
            let diff = EnvDiff::from_files(&start_or_attach.activation_state_dir)?;
            let mut start_services = assemble_command_for_activate_script(
                "activate_temporary",
                context.clone(),
                subsystem_verbosity,
                vars_from_env.clone(),
                &diff,
                &start_or_attach,
            );

            debug!("spawning activation services command: {:?}", start_services);
            start_services.spawn()?.wait()?;
        }

        attach(
            context,
            invocation_type,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
        )
    }
}

#[derive(Clone, Debug)]
pub struct VarsFromEnvironment {
    pub flox_env_dirs: Option<String>,
    pub path: String,
    pub manpath: Option<String>,
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
