use anyhow::{Context, Error};
use serde::Deserialize;
use tracing::debug;

use super::JobCtx;
use crate::generate::{copy_dir_recursive, run_cmd, run_post_cmd, run_pre_cmd, unpack_inputs};

#[derive(Debug, Clone, Deserialize)]
pub struct CustomJob {
    pub unpack_dir_contents: Option<Vec<String>>,
    pub ignore_errors: Option<bool>,
    pub pre_cmd: Option<String>,
    pub record_cmd: Option<String>,
    pub post_cmd: Option<String>,
}

pub fn run_custom_job(job: &CustomJob, ctx: &JobCtx) -> Result<(), Error> {
    debug!(category = ctx.category, name = ctx.name, "starting job");
    let workdir = ctx.tmp_dir.path();
    let vars = {
        let mut vars = ctx.vars.clone();
        vars.insert(
            "INPUT_DATA".to_string(),
            ctx.input_dir.to_string_lossy().to_string(),
        );
        vars
    };

    // Unpack and input directories to the workdir if they were specified
    if let Some(ref unpack_dir_contents) = job.unpack_dir_contents {
        unpack_inputs(&ctx.input_dir, unpack_dir_contents, workdir, ctx)
            .context("failed to unpack job inputs")?;
    }

    // Run the pre_cmd if it was specified
    if let Some(ref cmd) = job.pre_cmd {
        debug!(category = ctx.category, name = ctx.name, "running pre_cmd");
        run_pre_cmd(cmd, &vars, workdir, job.ignore_errors.unwrap_or(false))?;
    }

    // Run a command that will record a response if specified
    let resp_file = workdir.join("resp.yaml");
    if let Some(ref cmd) = job.record_cmd {
        debug!(
            category = ctx.category,
            name = ctx.name,
            "running record_cmd"
        );
        run_cmd(
            cmd,
            &vars,
            workdir,
            &resp_file,
            job.ignore_errors.unwrap_or(false),
        )?;
    }

    // Run the post_cmd if it was specified
    if let Some(ref cmd) = job.post_cmd {
        debug!(category = ctx.category, name = ctx.name, "running post_cmd");
        run_post_cmd(
            cmd,
            &vars,
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
