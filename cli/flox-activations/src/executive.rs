use anyhow::Result;
use flox_core::activate::context::{ActivateCtx, InvocationType};
use log::debug;
use nix::sys::signal::Signal::SIGUSR1;
use nix::sys::signal::kill;
use nix::unistd::Pid;

use crate::activate_script_builder::{
    assemble_command_for_activate_script,
    assemble_command_for_start_script,
};
use crate::cli::activate::VarsFromEnvironment;
use crate::cli::start_or_attach::StartOrAttachResult;
use crate::env_diff::EnvDiff;

pub fn executive(
    context: ActivateCtx,
    subsystem_verbosity: u32,
    vars_from_env: VarsFromEnvironment,
    start_or_attach: StartOrAttachResult,
    invocation_type: InvocationType,
    parent_pid: Pid,
) -> Result<(), anyhow::Error> {
    debug!("Starting activation");
    let mut start_command = assemble_command_for_start_script(
        context.clone(),
        subsystem_verbosity,
        vars_from_env.clone(),
        &start_or_attach,
        invocation_type,
    );
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

    kill(parent_pid, SIGUSR1)?;
    Ok(())
}
