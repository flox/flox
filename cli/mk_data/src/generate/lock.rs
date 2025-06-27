use std::fs::OpenOptions;
use std::process::{Command, Stdio};

use anyhow::{Context, Error};
use serde::Deserialize;
use tracing::debug;

use super::JobCtx;
use crate::generate::{JobCommand, err_with_stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct LockJob {
    pub manifest: String,
}

pub fn run_lock_job(job: &LockJob, ctx: &JobCtx) -> Result<(), Error> {
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
    let lockfile_path = workdir.join("manifest.lock");
    debug!(category = ctx.category, name = ctx.name, manifest = %manifest_path.display(), "flox lock-manifest");
    let output_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lockfile_path)
        .context("failed to create new lockfile")?;
    let output = Command::new("flox")
        .arg("lock-manifest")
        .arg(manifest_path)
        .current_dir(workdir)
        .stderr(Stdio::piped())
        .stdout(output_file)
        .apply_vars(&ctx.vars)
        .output()
        .context("failed to run `flox lock-manifest` command")?;
    err_with_stderr_if_err(output, false)?;

    // Copy the lockfile to the output directory
    let dest = ctx.category_dir.join(format!("{}.lock", ctx.name));
    debug!(category = ctx.category, name = ctx.name, src = %lockfile_path.display(), dest = %dest.display(), "moving lockfile");
    std::fs::copy(&lockfile_path, dest).context("failed to move lockfile")?;

    Ok(())
}
