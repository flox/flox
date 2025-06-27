use anyhow::{Context, Error};
use serde::Deserialize;
use tracing::debug;

use super::JobCtx;
use crate::generate::{
    JobCommand,
    copy_dir_recursive,
    run_post_cmd,
    run_pre_cmd,
    stderr_if_err,
    unpack_inputs,
};

#[derive(Debug, Clone, Deserialize)]
pub struct InitJob {
    pub unpack_dir_contents: Option<Vec<String>>,
    pub auto_setup: bool,
    pub ignore_errors: Option<bool>,
    pub pre_cmd: Option<String>,
    pub post_cmd: Option<String>,
}

pub fn run_init_job(job: &InitJob, ctx: &JobCtx) -> Result<(), Error> {
    debug!(category = ctx.category, name = ctx.name, "starting job");
    let workdir = ctx.tmp_dir.path();

    // Unpack and input directories to the workdir if they were specified
    if let Some(ref unpack_dir_contents) = job.unpack_dir_contents {
        unpack_inputs(&ctx.input_dir, unpack_dir_contents, workdir, ctx)
            .context("failed to unpack job inputs")?;
    }

    // Run the pre_cmd if it was specified
    if let Some(ref cmd) = job.pre_cmd {
        debug!(category = ctx.category, name = ctx.name, "running pre_cmd");
        run_pre_cmd(cmd, &ctx.vars, workdir, job.ignore_errors.unwrap_or(false))?;
    }

    // Create the environment
    debug!(category = ctx.category, name = ctx.name, dir = %workdir.display(), "flox init");
    let args = if job.auto_setup {
        vec!["init", "--auto-setup"]
    } else {
        vec!["init"]
    };
    let resp_file = workdir.join("resp.yaml");
    let cmd = duct::cmd("flox", args)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file);
    let output = cmd.run().context("failed to run `flox init` command")?;
    stderr_if_err(output)?;

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

    // Copy the contents of the working directory to `test_data/<category>/<name>`
    debug!(
        category = ctx.category,
        name = ctx.name,
        "moving to output directory"
    );
    let output_dir = ctx.category_dir.join(&ctx.name);
    copy_dir_recursive(workdir, &output_dir).context("failed to copy to output directory")?;

    Ok(())
}
