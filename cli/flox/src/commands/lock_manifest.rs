use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bpaf::Bpaf;
use flox_manifest::Manifest;
use flox_manifest::interfaces::AsTypedOnlyManifest;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::fetcher::IncludeFetcher;
use tracing::instrument;

use crate::commands::SHELL_COMPLETION_FILE;
use crate::subcommand_metric;

/// Lock a manifest file read from the path specified or stdin if `-`.
/// If provided, uses the lockfile from the path specified by `--lockfile`
/// as the base lockfile.
/// Returns the lockfile as JSON to stdout.
/// Manifests with includes cannot be locked.
#[derive(Bpaf, Clone)]
pub struct LockManifest {
    /// The previous lockfile to use as a base.
    #[bpaf(long, short, argument("path"), complete_shell(SHELL_COMPLETION_FILE))]
    lockfile: Option<PathBuf>,

    /// The manifest file to lock. (default: stdin)
    #[bpaf(positional("path to manifest"), complete_shell(SHELL_COMPLETION_FILE))]
    manifest: PathBuf,
}

impl LockManifest {
    #[instrument(name = "lock", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("lock");

        let manifest_path = if self.manifest == Path::new("-") {
            Path::new("/dev/stdin")
        } else {
            &self.manifest
        };

        let input_manifest =
            fs::read_to_string(manifest_path).context("Failed to read manifest file")?;

        let input_manifest = Manifest::parse_toml_typed(input_manifest)?;

        let input_lockfile = if let Some(lockfile_path) = self.lockfile {
            let lockfile = fs::read_to_string(lockfile_path).context("Failed to read lockfile")?;
            Some(serde_json::from_str(&lockfile).context("Failed to parse lockfile")?)
        } else {
            None
        };

        let migrated_manifest = input_manifest
            .as_typed_only()
            .migrate_typed_only(input_lockfile.as_ref())?;

        let lockfile = flox_rust_sdk::providers::lock_manifest::LockManifest::lock_manifest(
            &flox,
            &migrated_manifest,
            input_lockfile.as_ref(),
            // For now this will just cause an error if the manifest has includes
            &IncludeFetcher {
                base_directory: None,
            },
        )
        .await
        .context("Failed to lock the manifest")?;

        serde_json::to_writer_pretty(std::io::stdout(), &lockfile)
            .context("failed to write lockfile to stdout")?;
        Ok(())
    }
}
