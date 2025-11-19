use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::auth::{NixCopyAuth, write_floxhub_netrc};
use flox_rust_sdk::providers::publish::ClientSideCatalogStoreConfig;
use tracing::instrument;
use url::Url;

use crate::commands::{SHELL_COMPLETION_FILE, ensure_floxhub_token};
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Upload {
    #[bpaf(external(cache_args))]
    cache: CacheArgs,

    #[bpaf(positional("store-path"))]
    store_path: PathBuf,
}

#[derive(Debug, Bpaf, Clone)]
struct CacheArgs {
    /// URL of store to copy packages to.
    #[bpaf(long, argument("URL"))]
    store_url: Url,

    /// Path of the key file used to sign packages before copying.
    #[bpaf(long, argument("FILE"), complete_shell(SHELL_COMPLETION_FILE))]
    signing_key: PathBuf,
}

impl Upload {
    #[instrument(name = "upload", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        if !flox.features.upload {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'upload' feature is not enabled.");
        }

        subcommand_metric!("upload");

        let store_path = validate_store_path(self.store_path.clone())?;
        let token = ensure_floxhub_token(&mut flox).await?.clone();
        let auth_file = write_floxhub_netrc(flox.temp_dir, &token)?;

        ClientSideCatalogStoreConfig::upload_store_path(
            &self.cache.store_url,
            Some(self.cache.signing_key.as_path()),
            &Some(NixCopyAuth::Netrc(auth_file.to_path_buf())),
            &store_path.to_string_lossy(),
        )
        .context("Failed to upload artifact")?;

        message::updated(format!(
            "Store path {} uploaded successfully.",
            store_path.display()
        ));

        Ok(())
    }
}

fn validate_store_path(store_path: PathBuf) -> Result<PathBuf> {
    if !store_path.exists() {
        bail!("Store path does not exist: {}", store_path.display());
    }

    let store_path = store_path.canonicalize()?;

    if !store_path.starts_with("/nix/store/") {
        bail!(
            "Store path is not in the Nix store: {}",
            store_path.display()
        );
    }

    Ok(store_path)
}

#[cfg(test)]
mod test {
    use flox_rust_sdk::providers::nix::test_helpers::known_store_path;

    #[test]
    fn validate_store_path_nonexistent_file() {
        let store_path = std::path::PathBuf::from("/nix/store/nonexistent-store-path");
        let result = super::validate_store_path(store_path);
        assert!(result.is_err());
    }

    #[test]
    fn validate_store_path_non_nix_path() {
        let tempfile = tempfile::NamedTempFile::new().unwrap();
        let store_path = tempfile.path().into();
        let result = super::validate_store_path(store_path);
        assert!(result.is_err());
    }

    #[test]
    fn validate_store_path_invalid_symlink() {
        let store_path = std::path::PathBuf::from("/nix/store/nonexistent-store-path");
        let tempdir = tempfile::tempdir().unwrap();
        let symlink = tempdir.path().join("test-link");
        std::os::unix::fs::symlink(&store_path, &symlink).unwrap();
        let result = super::validate_store_path(symlink);
        assert!(result.is_err());
    }

    #[test]
    fn validate_store_path_follow_symlinks() {
        let store_path = known_store_path();
        let tempdir = tempfile::tempdir().unwrap();
        let symlink = tempdir.path().join("test-link");
        std::os::unix::fs::symlink(&store_path, &symlink).unwrap();
        let result = super::validate_store_path(symlink);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), store_path);
    }
}
