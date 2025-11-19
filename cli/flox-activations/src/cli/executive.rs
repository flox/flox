use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use clap::Args;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use log::debug;
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::{Pid, sleep};
use serde::{Deserialize, Serialize};

use crate::activate_script_builder::{
    assemble_command_for_activate_script,
    assemble_command_for_start_script,
};
use crate::cli::activate::{NO_REMOVE_ACTIVATION_FILES, VarsFromEnvironment};
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutiveCtx {
    pub context: ActivateCtx,
    pub subsystem_verbosity: u32,
    pub vars_from_env: VarsFromEnvironment,
    pub start_or_attach: StartOrAttachResult,
    pub invocation_type: InvocationType,
    pub parent_pid: i32,
}

#[derive(Debug, Args)]
pub struct ExecutiveArgs {
    /// Path to JSON file containing executive context
    #[arg(long)]
    pub executive_ctx: PathBuf,
}

impl ExecutiveArgs {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        let contents = fs::read_to_string(&self.executive_ctx)?;
        let ExecutiveCtx {
            context,
            subsystem_verbosity,
            vars_from_env,
            start_or_attach,
            invocation_type,
            parent_pid,
        } = serde_json::from_str(&contents)?;
        if !std::env::var(NO_REMOVE_ACTIVATION_FILES).is_ok_and(|val| val == "true") {
            fs::remove_file(&self.executive_ctx)?;
        }

        let mut start_command = assemble_command_for_start_script(
            context.clone(),
            subsystem_verbosity,
            vars_from_env.clone(),
            &start_or_attach,
            invocation_type,
        );
        debug!("spawning start.bash: {:?}", start_command);
        start_command.spawn()?.wait()?;
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
        };

        // Signal the parent that activation is ready
        debug!("sending SIGUSR1 to parent {}", parent_pid);
        kill(Pid::from_raw(parent_pid), SIGUSR1)?;

        // TODO: once we wait for activations to exit, we can remove this,
        // but at this point flox-activations activate may receive SIGCHLD
        // before SIGUSR1 if we don't wait around a bit
        sleep(1);
        Ok(())
    }
}
