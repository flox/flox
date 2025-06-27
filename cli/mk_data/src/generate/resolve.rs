use std::process::Command;

use anyhow::{Context, Error};
use serde::Deserialize;
use tracing::debug;

use super::JobCtx;
use crate::generate::{
    JobCommand,
    err_with_stderr_if_err,
    move_response_file,
    run_post_cmd,
    run_pre_cmd,
};

#[derive(Debug, Clone, Deserialize)]
pub struct ResolveJob {
    pub pkgs: Vec<String>,
    pub ignore_errors: Option<bool>,
    pub pre_cmd: Option<String>,
    pub post_cmd: Option<String>,
}

pub fn run_resolve_job(job: &ResolveJob, ctx: &JobCtx) -> Result<(), Error> {
    debug!(category = ctx.category, name = ctx.name, "starting job");
    let workdir = ctx.tmp_dir.path();

    // Create the environment
    debug!(category = ctx.category, name = ctx.name, dir = %workdir.display(), "flox init");
    let output = Command::new("flox")
        .arg("init")
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .output()
        .context("failed to run `flox init` command")?;
    err_with_stderr_if_err(output, job.ignore_errors.unwrap_or(false))?;

    // Run the pre_cmd if it was specified
    if let Some(ref cmd) = job.pre_cmd {
        debug!(category = ctx.category, name = ctx.name, "running pre_cmd");
        run_pre_cmd(cmd, &ctx.vars, workdir, job.ignore_errors.unwrap_or(false))?;
    }

    // Run the install command and capture the response
    debug!(category = ctx.category, name = ctx.name, "flox install");
    let resp_file = workdir.join("resp.yaml");
    let args = {
        let mut args = vec!["install".to_string()];
        args.extend_from_slice(job.pkgs.as_slice());
        args
    };
    let output = Command::new("flox")
        .args(args)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file)
        .output()
        .context("failed to run `flox install` command")?;
    err_with_stderr_if_err(output, job.ignore_errors.unwrap_or(false))?;

    // Run the post_cmd if it was specified
    if let Some(ref cmd) = job.post_cmd {
        debug!(category = ctx.category, name = ctx.name, "running post_cmd");
        run_post_cmd(
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
