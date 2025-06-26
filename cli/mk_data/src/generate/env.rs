use std::path::Path;

use anyhow::{Context, Error};
use duct::cmd;
use serde::Deserialize;
use tracing::debug;

use super::JobCtx2;
use crate::generate::{JobCommand, copy_dir_recursive, stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct EnvJob {
    pub manifest: String,
}

pub fn run_env_job(job: &EnvJob, ctx: &JobCtx2, input_data_dir: &Path) -> Result<(), Error> {
    debug!(category = ctx.category, name = ctx.name, "starting job");
    let workdir = ctx.tmp_dir.path();

    // Create the environment
    debug!(category = ctx.category, name = ctx.name, dir = %workdir.display(), "flox init");
    let cmd = cmd!("flox", "init")
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars);
    let output = cmd.run().context("failed to run `flox init` command")?;
    stderr_if_err(output)?;

    // Build the environment with the new manifest
    let manifest_path = input_data_dir.join("manifests").join(&job.manifest);
    let resp_file = workdir.join("resp.yaml");
    debug!(category = ctx.category, name = ctx.name, manifest = %manifest_path.display(), "flox edit -f");
    let output = cmd!("flox", "edit", "-f", manifest_path)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file)
        .run()
        .context("failed to run `flox edit -f` command")?;
    stderr_if_err(output)?;

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
