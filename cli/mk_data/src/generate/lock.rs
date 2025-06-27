use anyhow::{Context, Error};
use duct::cmd;
use serde::Deserialize;
use tracing::debug;

use super::JobCtx2;
use crate::generate::{JobCommand, stderr_if_err};

#[derive(Debug, Clone, Deserialize)]
pub struct LockJob {
    pub manifest: String,
}

pub fn run_lock_job(job: &LockJob, ctx: &JobCtx2) -> Result<(), Error> {
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
    let manifest_path = ctx.input_dir.join("manifests").join(&job.manifest);
    let lockfile_path = workdir.join("manifest.lock");
    debug!(category = ctx.category, name = ctx.name, manifest = %manifest_path.display(), "flox lock-manifest");
    let output = cmd!("flox", "lock-manifest", manifest_path)
        .dir(workdir)
        .stderr_capture()
        .stdout_path(&lockfile_path)
        .apply_vars(&ctx.vars)
        .run()
        .context("failed to run `flox lock-manifest` command")?;
    stderr_if_err(output)?;

    // Copy the lockfile to the output directory
    let dest = ctx.category_dir.join(format!("{}.lock", ctx.name));
    debug!(category = ctx.category, name = ctx.name, src = %lockfile_path.display(), dest = %dest.display(), "moving lockfile");
    std::fs::copy(&lockfile_path, dest).context("failed to move lockfile")?;

    Ok(())
}
