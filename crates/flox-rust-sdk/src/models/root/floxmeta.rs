use std::path::PathBuf;

use derive_more::Constructor;
use futures::{StreamExt, TryStreamExt};
use log::warn;
use runix::command::FlakeInit;
use runix::{NixBackend, Run};
use thiserror::Error;

use super::reference::ProjectDiscoverGitError;
use super::{Closed, Root};
use crate::flox::Flox;
use crate::providers::git::GitProvider;

pub const FLOXMETA_DIR_NAME: &str = "meta";

#[derive(Constructor, Debug)]
pub struct Floxmeta<'flox, T> {
    pub git: T,
    pub(crate) flox: &'flox Flox,
}

impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Guards opening a repo as floxmeta
    /// Ensures that the repo is in fact a floxmeta directory
    pub async fn guard_floxmeta(self) -> Result<Floxmeta<'flox, Git>, OpenFloxmetaError> {
        Ok(Floxmeta {
            git: self.state.inner,
            flox: self.flox,
        })
    }
}

/// Constructors and implementations for floxmeta
impl<'flox, Git: GitProvider> Floxmeta<'flox, Git> {
    /// lists all floxmeta repositories currently cloned to the floxmeta cache
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - the cache dir cannot be opened or created
    /// - any **directory** in the cache dir cannot be resolved as a floxmeta repo
    pub async fn list_floxmetas(
        flox: &'flox Flox,
    ) -> Result<Vec<Floxmeta<'flox, Git>>, ListFloxmetaError<Git>> {
        let metadir = flox.cache_dir.join(FLOXMETA_DIR_NAME);

        if !metadir.exists() {
            tokio::fs::create_dir_all(&metadir)
                .await
                .map_err(|e| ListFloxmetaError::CreateMetaDir(metadir.clone(), e))?
        }

        let floxmeta_dirs = futures::stream::iter(
            metadir
                .read_dir()
                .map_err(|e| ListFloxmetaError::OpenMetaDir(metadir.clone(), e))?,
        )
        .filter_map(|entry| async {
            entry.map_or_else(
                |e| {
                    warn!("Could not read a floxmeta cache entry in {metadir:?}, ignoring: {e}");
                    None
                },
                Some,
            )
        })
        .filter_map(|entry| async {
            match entry.file_type() {
                Ok(t) if t.is_symlink() => None,
                Ok(_) => Some(entry),
                Err(e) => {
                    warn!(
                        "Could not determine type of {path:?}, ignoring: {e}",
                        path = entry.path()
                    );
                    None
                },
            }
        })
        .then(|dir| async move {
            let owner = dir.file_name().to_string_lossy().into_owned();
            let floxmeta = Self::get_floxmeta(flox, &owner).await?;
            Ok(floxmeta)
        })
        .try_collect()
        .await;

        floxmeta_dirs
    }

    /// gets a floxmeta reference for a specific owner
    ///
    /// Looks up a directory in $CACHE_DIR/meta
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - the floxmeta dir does not exist
    /// - the floxmeta dir cannot be opened as a git repo
    /// - the floxmeta dir is not valid
    pub async fn get_floxmeta(
        flox: &'flox Flox,
        owner: &str,
    ) -> Result<Floxmeta<'flox, Git>, GetFloxmetaError<Git>> {
        let floxmeta_dir = flox.cache_dir.join(FLOXMETA_DIR_NAME).join(owner);
        let git = flox
            .project(floxmeta_dir.clone())
            .guard::<Git>()
            .await
            .map_err(|e| GetFloxmetaError::DiscoverGitDir(floxmeta_dir, e))?
            .ensure(|_| Err(GetFloxmetaError::NotFound(owner.to_string())))?;

        let floxmeta = git.guard_floxmeta().await?;

        Ok(floxmeta)
    }
}

/// Errors occurring while trying to upgrade to an [`Open<Git>`] [Root]
#[derive(Error, Debug)]
pub enum OpenFloxmetaError {
    #[error("Could not determine repository root")]
    WorkdirNotFound,
}

#[derive(Error, Debug)]
pub enum InitFloxmetaError<Nix: NixBackend, Git: GitProvider>
where
    FlakeInit: Run<Nix>,
{
    #[error("Could not determine repository root")]
    WorkdirNotFound,

    #[error("Error initializing base template with Nix")]
    NixInitBase(<FlakeInit as Run<Nix>>::Error),
    #[error("Error reading template file contents")]
    ReadTemplateFile(std::io::Error),
    #[error("Error truncating template file")]
    TruncateTemplateFile(std::io::Error),
    #[error("Error writing to template file")]
    WriteTemplateFile(std::io::Error),
    #[error("Error new template file in Git")]
    GitAdd(Git::AddError),
}

#[derive(Error, Debug)]
pub enum ListFloxmetaError<Git: GitProvider> {
    #[error("Could not open floxmeta cache ({0:?}): ({1})")]
    OpenMetaDir(PathBuf, std::io::Error),

    #[error("Could not create floxmeta cache ({0:?}): ({1})")]
    CreateMetaDir(PathBuf, std::io::Error),

    #[error(transparent)]
    GetFloxmeta(#[from] GetFloxmetaError<Git>),
}

#[derive(Error, Debug)]
pub enum GetFloxmetaError<Git: GitProvider> {
    #[error("Error opening floxmeta dir for ({0:?}): ({1})")]
    DiscoverGitDir(PathBuf, ProjectDiscoverGitError<Git>),

    #[error("Error opening floxmeta dir for {0:?}: Not found")]
    NotFound(String),

    #[error(transparent)]
    OpenFloxmeta(#[from] OpenFloxmetaError),
}

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
mod floxmeta_tests {
    use tempfile::TempDir;

    use super::*;
    use crate::providers::git::GitCommandProvider;

    fn flox_instance() -> (Flox, TempDir) {
        let tempdir = tempfile::tempdir().unwrap();

        let flox = Flox {
            system: "aarch64-darwin".to_string(),
            cache_dir: tempdir.path().to_owned(),
            ..Default::default()
        };

        (flox, tempdir)
    }

    #[tokio::test]
    async fn fail_without_metadir() {
        let (flox, _) = flox_instance();

        let floxmeta = Floxmeta::<GitCommandProvider>::get_floxmeta(&flox, "someone").await;

        assert!(matches!(floxmeta, Err(_)));
    }

    #[tokio::test]
    async fn test_reading_remote_envs() {
        let (flox, cache) = flox_instance();

        let meta_repo = cache.path().join(FLOXMETA_DIR_NAME).join("flox");
        tokio::fs::create_dir_all(&meta_repo).await.unwrap();

        let git = GitCommandProvider::init(&meta_repo, true).await.unwrap();

        let floxmeta = Floxmeta::<GitCommandProvider>::get_floxmeta(&flox, "flox")
            .await
            .expect("Should open floxmeta repo");
        let environments = floxmeta
            .environments()
            .await
            .expect("Should succeed with zero environment");

        assert!(environments.is_empty());

        git.add_remote("origin", "https://github.com/flox/floxmeta")
            .await
            .expect("Failed adding origin");
        git.fetch().await.expect("Failed fetching origin");

        let environments = floxmeta
            .environments()
            .await
            .expect("Should succeed with zero environment");

        assert!(!environments.is_empty());
    }
}
