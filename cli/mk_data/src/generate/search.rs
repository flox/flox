use anyhow::{Context, Error};
use duct::cmd;
use serde::Deserialize;
use tracing::debug;

use super::JobCtx2;
use crate::generate::{JobCommand, move_response_file, run_post_cmd2, run_pre_cmd2, stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct SearchJob {
    pub query: String,
    pub all: Option<bool>,
    pub ignore_errors: Option<bool>,
    pub pre_cmd: Option<String>,
    pub post_cmd: Option<String>,
}

pub fn run_search_job(job: &SearchJob, ctx: &JobCtx2) -> Result<(), Error> {
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
    debug!(category = ctx.category, name = ctx.name, "flox search");
    let resp_file = workdir.join("resp.yaml");
    let args = if job.all.unwrap_or(false) {
        vec!["search".to_string(), job.query.clone(), "--all".to_string()]
    } else {
        vec!["search".to_string(), job.query.clone()]
    };
    let maybe_output = duct::cmd("flox", args)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file)
        .unchecked()
        .run()
        .context("failed to run `flox search` command");
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

    // Move the response file to `test_data/search/<name>.yaml`
    move_response_file(&resp_file, ctx)?;

    Ok(())
}
