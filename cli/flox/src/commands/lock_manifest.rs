use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::lockfile::Lockfile;
use tracing::instrument;

use crate::subcommand_metric;

/// Lock a manifest file read from the path specified or stdin if `-`.
/// If provided, uses the lockfile from the path specified by `--lockfile`
/// as the base lockfile.
/// Returns the lockfile as JSON to stdout.
#[derive(Bpaf, Clone)]
pub struct LockManifest {
    /// The previous lockfile to use as a base.
    #[bpaf(long, short, argument("path"))]
    lockfile: Option<PathBuf>,

    /// The manifest file to lock. (default: stdin)
    #[bpaf(positional("path to manifest"))]
    manifest: PathBuf,
}

impl LockManifest {
    #[instrument(name = "lock", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("lock");

        let manifest_path = if self.manifest == PathBuf::from("-") {
            Path::new("/dev/stdin")
        } else {
            &self.manifest
        };

        let input_manifest =
            fs::read_to_string(manifest_path).context("Failed to read manifest file")?;

        let input_manifest =
            toml::from_str(&input_manifest).context("Failed to parse manifest file")?;

        let input_lockfile = if let Some(lockfile_path) = self.lockfile {
            let lockfile = fs::read_to_string(lockfile_path).context("Failed to read lockfile")?;
            Some(serde_json::from_str(&lockfile).context("Failed to parse lockfile")?)
        } else {
            None
        };

        let lockfile = Lockfile::lock_manifest(
            &input_manifest,
            input_lockfile.as_ref(),
            &flox.catalog_client,
            &flox.installable_locker,
            flox.features.compose,
        )
        .await
        .context("Failed to lock the manifest")?;

        serde_json::to_writer_pretty(std::io::stdout(), &lockfile)
            .context("failed to write lockfile to stdout")?;
        Ok(())
    }
}
