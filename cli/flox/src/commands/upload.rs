use std::path::PathBuf;

use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::publish::{BinaryCache, NixCopyCache};
use tracing::instrument;
use url::Url;

use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Upload {
    #[bpaf(external(cache_args))]
    cache: CacheArgs,

    #[bpaf(external(upload_store_path))]
    store_path: UploadStorePath,
}

#[derive(Debug, Bpaf, Clone)]
struct CacheArgs {
    #[bpaf(long("cache"))]
    url: Url,

    #[bpaf(long("signing-key"))]
    key_file: PathBuf,
}

#[derive(Debug, Bpaf, Clone)]
struct UploadStorePath {
    /// The store path to upload.
    #[bpaf(positional("store-path"))]
    store_path: PathBuf,
}

impl Upload {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !config.features.unwrap_or_default().upload {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'upload' feature is not enabled.");
        }

        let UploadStorePath { store_path } = self.store_path;

        Self::upload(flox, store_path, self.cache).await
    }

    #[instrument(name = "upload", skip_all, fields(package))]
    async fn upload(mut _flox: Flox, store_path: PathBuf, cache_args: CacheArgs) -> Result<()> {
        subcommand_metric!("upload");

        let store_path = validate_store_path(store_path)?;

        let cache = NixCopyCache {
            url: cache_args.url,
            key_file: cache_args.key_file,
        };

        let result = Dialog {
            message: &format!("Uploading store path {}...", store_path.display()),
            help_message: None,
            typed: Spinner::new(|| cache.upload(&store_path.to_string_lossy())),
        }
        .spin();
        match result {
            Ok(_) => message::updated(format!(
                "Store path {} uploaded successfully.",
                store_path.display()
            )),
            Err(e) => bail!("Failed to upload artifact: {}", e.to_string()),
        }

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

mod test {
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
    fn validate_store_path_folow_symlinks() {
        let store_path = std::path::PathBuf::from(env!("NIX_BIN"));
        let tempdir = tempfile::tempdir().unwrap();
        let symlink = tempdir.path().join("test-link");
        std::os::unix::fs::symlink(&store_path, &symlink).unwrap();
        let result = super::validate_store_path(symlink);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), store_path);
    }
}
