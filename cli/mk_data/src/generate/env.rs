use std::process::Command;

use anyhow::{Context, Error};
use serde::Deserialize;
use tracing::debug;

use super::JobCtx;
use crate::generate::{JobCommand, copy_dir_recursive, err_with_stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct EnvJob {
    pub manifest: String,
}

pub fn run_env_job(job: &EnvJob, ctx: &JobCtx) -> Result<(), Error> {
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
    err_with_stderr_if_err(output, false)?;

    // Build the environment with the new manifest
    let manifest_path = ctx.input_dir.join("manifests").join(&job.manifest);
    let resp_file = workdir.join("resp.yaml");
    debug!(category = ctx.category, name = ctx.name, manifest = %manifest_path.display(), "flox edit -f");
    let output = Command::new("flox")
        .arg("edit")
        .arg("-f")
        .arg(manifest_path)
        .apply_common_options(workdir)
        .apply_vars(&ctx.vars)
        .apply_recording_vars(&resp_file)
        .output()
        .context("failed to run `flox edit -f` command")?;
    err_with_stderr_if_err(output, false)?;

    // Copy the contents of the working directory to `test_data/<category>/<name>`
    debug!(
        category = ctx.category,
        name = ctx.name,
        "moving to output directory"
    );
    let output_dir = ctx.category_dir.join(&ctx.name);
    if output_dir.exists() {
        std::fs::remove_dir_all(&output_dir).with_context(|| {
            format!(
                "failed to remove existing output directory: {}",
                output_dir.display()
            )
        })?;
    }
    copy_dir_recursive(workdir, &output_dir).context("failed to copy to output directory")?;

    Ok(())
}
