use anyhow::{Context, Error};
use duct::cmd;
use serde::Deserialize;
use tracing::debug;

use super::JobCtx2;
use crate::generate::{JobCommand, move_response_file, run_post_cmd2, run_pre_cmd2, stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveJob {
    pub pkgs: Vec<String>,
    pub ignore_errors: Option<bool>,
    pub pre_cmd: Option<String>,
    pub post_cmd: Option<String>,
}

pub fn run_resolve_job(job: &ResolveJob, ctx: &JobCtx2) -> Result<(), Error> {
    debug!(category = ctx.category, name = ctx.name, "starting job");
    let workdir = ctx.tmp_dir.path();

    // Create the environment
    debug!(category = ctx.category, name = ctx.name, dir = %workdir.display(), "flox init");
    let cmd = cmd!("flox", "init")
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars);
    let output = cmd.run().context("failed to run `flox init` command")?;
    stderr_if_err(output)?;

    // Run the pre_cmd if it was specified
    if let Some(ref cmd) = job.pre_cmd {
        debug!(category = ctx.category, name = ctx.name, "running pre_cmd");
        run_pre_cmd2(cmd, &ctx.vars, workdir, job.ignore_errors.unwrap_or(false))?;
    }

    // Run the install command and capture the response
    debug!(category = ctx.category, name = ctx.name, "flox install");
    let resp_file = workdir.join("resp.yaml");
    let args = {
        let mut args = vec!["install".to_string()];
        args.extend_from_slice(job.pkgs.as_slice());
        args
    };
    let maybe_output = duct::cmd("flox", args)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file)
        .unchecked()
        .run()
        .context("failed to run `flox install` command");
    if !job.ignore_errors.unwrap_or(false) {
        let output = maybe_output?;
        stderr_if_err(output)?;
    }

    // Run the post_cmd if it was specified
    if let Some(ref cmd) = job.post_cmd {
        debug!(category = ctx.category, name = ctx.name, "running post_cmd");
        run_post_cmd2(
            cmd,
            &ctx.vars,
            workdir,
            &resp_file,
            job.ignore_errors.unwrap_or(false),
        )?;
    }

    // Move the response file to `test_data/resolve/<name>.yaml`
    move_response_file(&resp_file, ctx)?;

    Ok(())
}
