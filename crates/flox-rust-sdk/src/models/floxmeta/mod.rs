use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use futures::{StreamExt, TryStreamExt};
use log::warn;
use runix::command::FlakeInit;
use runix::{NixBackend, Run};
use tempfile::TempDir;
use thiserror::Error;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

pub mod environment;
use environment::{Metadata, METADATA_JSON};

use super::root::reference::ProjectDiscoverGitError;
use super::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use super::root::{Closed, Root};
use crate::flox::Flox;
use crate::providers::git::GitProvider;

pub const FLOXMETA_DIR_NAME: &str = "meta";

#[derive(Debug)]
pub struct Floxmeta<'flox, Git: GitProvider, Access: GitAccess<Git>> {
    pub(crate) flox: &'flox Flox,
    pub(crate) owner: String,

    pub(crate) access: Access,
    pub(crate) _git: PhantomData<Git>,
}

impl<'flox, Git: GitProvider> Root<'flox, Closed<Git>> {
    /// Guards opening a repo as floxmeta
    ///
    /// - Ensures that the repo is in fact a floxmeta directory
    /// - resolves the environment owner from the git workdir
    ///
    /// ## Discussion
    ///
    /// - if in the future these repositories are places in other places,
    ///   without provenance of the owner,
    ///   this guard should take the owner as an argument instead.
    pub async fn guard_floxmeta(
        self,
    ) -> Result<Floxmeta<'flox, Git, ReadOnly<Git>>, OpenFloxmetaError> {
        let owner = self
            .state
            .inner
            .workdir()
            .unwrap_or_else(|| self.state.inner.path())
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap();

        Ok(Floxmeta {
            owner: owner.into_owned(),
            flox: self.flox,
            access: ReadOnly::new(self.state.inner),
            _git: PhantomData::default(),
        })
    }
}

/// Constructors and implementations for retrieving floxmeta handles
/// and creating a writable transaction
impl<'flox, Git: GitProvider> Floxmeta<'flox, Git, ReadOnly<Git>> {
    /// lists all floxmeta repositories currently cloned to the floxmeta cache
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - the cache dir cannot be opened or created
    /// - any **directory** in the cache dir cannot be resolved as a floxmeta repo
    pub async fn list_floxmetas(
        flox: &'flox Flox,
    ) -> Result<Vec<Floxmeta<'flox, Git, ReadOnly<Git>>>, ListFloxmetaError<Git>> {
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
    /// Looks up a directory in $CAHCE_DIR/meta
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
    ) -> Result<Floxmeta<'flox, Git, ReadOnly<Git>>, GetFloxmetaError<Git>> {
        let floxmeta_dir = flox.cache_dir.join(FLOXMETA_DIR_NAME).join(owner);
        let git = flox
            .resource(floxmeta_dir.clone())
            .guard::<Git>()
            .await
            .map_err(|e| GetFloxmetaError::DiscoverGitDir(floxmeta_dir, e))?
            .ensure(|_| Err(GetFloxmetaError::NotFound(owner.to_string())))?;

        let floxmeta = git.guard_floxmeta().await?;

        Ok(floxmeta)
    }

    pub async fn enter_transaction(
        self,
    ) -> Result<Floxmeta<'flox, Git, GitSandBox<Git>>, TransactionEnterError<Git>> {
        let transaction_temp_dir =
            TempDir::new_in(&self.flox.temp_dir).map_err(TransactionEnterError::CreateTempdir)?;

        let transaction_git =
            Git::clone(self.access.git().path(), transaction_temp_dir.path(), false)
                .await
                .map_err(TransactionEnterError::GitClone)?;

        let sandbox = self
            .access
            .to_sandbox_in(transaction_temp_dir, transaction_git);

        Ok(Floxmeta {
            owner: self.owner.clone(),
            flox: self.flox,
            access: sandbox,
            _git: PhantomData::default(),
        })
    }
}

/// Constructors and implementations for retrieving floxmeta handles
/// and creating a writable transaction
impl<'flox, Git: GitProvider, Access: GitAccess<Git>> Floxmeta<'flox, Git, Access> {
    pub fn owner(&self) -> &str {
        &self.owner
    }

    pub fn git(&self) -> &Git {
        self.access.git()
    }
}

/// Constructors and implementations for writable (sandbox) floxmeta
impl<'flox, Git: GitProvider> Floxmeta<'flox, Git, GitSandBox<Git>> {
    /// abort a transaction by discarding the temporary clone
    pub async fn abort_transaction(self) -> Floxmeta<'flox, Git, ReadOnly<Git>> {
        let access = self.access.abort();
        Floxmeta {
            owner: self.owner,
            flox: self.flox,
            access,
            _git: PhantomData::default(),
        }
    }

    /// complete floxmeta transaction by committing staged changes and pushing to origin
    pub async fn commit_transaction(
        self,
        message: &str,
    ) -> Result<Floxmeta<'flox, Git, ReadOnly<Git>>, TransactionCommitError<Git>> {
        self.access
            .git()
            .commit(message)
            .await
            .map_err(TransactionCommitError::GitCommit)?;
        self.access
            .git()
            .push("origin")
            .await
            .map_err(TransactionCommitError::GitPush)?;

        Ok(Floxmeta {
            owner: self.owner,
            flox: self.flox,
            access: self.access.abort(),
            _git: PhantomData::default(),
        })
    }

    pub async fn create_environment(&self, name: &str) -> Result<(), CreateEnvironmentError<Git>> {
        // todo make Self::environment(&self, name) produce a guard?
        if self.environment(name).await.is_ok() {
            return Err(CreateEnvironmentError::EnvironmentExists(
                name.to_string(),
                self.flox.system.to_string(),
            ));
        }

        let branch_name = format!("{}.{}", self.flox.system, name);
        self.access
            .git()
            .checkout(&branch_name, true)
            .await
            .map_err(|e| CreateEnvironmentError::<Git>::GitCheckout(branch_name, e))?;

        let metadata = Metadata::default();

        let mut metadata_json = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(
                self.access
                    .git()
                    .workdir()
                    .expect("Workdir should exist during transaction")
                    .join(METADATA_JSON),
            )
            .await
            .map_err(CreateEnvironmentError::OpenMetadataFile)?;

        metadata_json
            .write_all(
                serde_json::to_string_pretty(&metadata)
                    .map_err(CreateEnvironmentError::SerializeMetadata)?
                    .as_bytes(),
            )
            .await
            .map_err(CreateEnvironmentError::WriteMetadata)?;

        self.access
            .git()
            .add(&[Path::new(METADATA_JSON)])
            .await
            .map_err(CreateEnvironmentError::GitAdd)?;
        Ok(())
    }
}

/// Errors occuring while trying to upgrade to an [`Open<Git>`] [Root]
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

#[derive(Error, Debug)]
pub enum CreateEnvironmentError<Git: GitProvider> {
    #[error("Environment '{0}' ({1}) already exists")]
    EnvironmentExists(String, String),

    #[error("Failed checking out branch '{0}': {1}")]
    GitCheckout(String, Git::CheckoutError),

    #[error("Failed to open metadata file for writing: {0}")]
    OpenMetadataFile(std::io::Error),

    #[error("Failed serializing metadata: {0}")]
    SerializeMetadata(#[from] serde_json::Error),

    #[error("Failed writing metadata: {0}")]
    WriteMetadata(std::io::Error),

    #[error("Failed adding metadata to git index: {0}")]
    GitAdd(Git::AddError),
}

#[derive(Error, Debug)]
pub enum TransactionEnterError<Git: GitProvider> {
    #[error("Failed to create tempdir for transaction")]
    CreateTempdir(std::io::Error),
    #[error("Failed to clone env into tempdir")]
    GitClone(Git::CloneError),
}
#[derive(Error, Debug)]
pub enum TransactionCommitError<Git: GitProvider> {
    GitCommit(Git::CommitError),
    GitPush(Git::PushError),
}

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
mod floxmeta_tests {
    use tempfile::TempDir;

    use super::*;
    use crate::providers::git::GitCommandProvider;

    fn flox_instance() -> (Flox, TempDir) {
        let tempdir_handle = tempfile::tempdir_in(std::env::temp_dir()).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let temp_dir = tempdir_handle.path().join("temp");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();

        let flox = Flox {
            system: "aarch64-darwin".to_string(),
            cache_dir,
            temp_dir,
            ..Default::default()
        };

        (flox, tempdir_handle)
    }

    #[tokio::test]
    async fn fail_without_metadir() {
        let (flox, _tempdir_handle) = flox_instance();

        let floxmeta =
            Floxmeta::<GitCommandProvider, ReadOnly<_>>::get_floxmeta(&flox, "someone").await;

        assert!(matches!(floxmeta, Err(_)));
    }

    #[tokio::test]
    async fn test_reading_remote_envs() {
        let (flox, _tempdir_handle) = flox_instance();

        let meta_repo = flox.cache_dir.join(FLOXMETA_DIR_NAME).join("flox");
        tokio::fs::create_dir_all(&meta_repo).await.unwrap();

        let git = GitCommandProvider::init(&meta_repo, true).await.unwrap();

        let floxmeta = Floxmeta::<GitCommandProvider, ReadOnly<_>>::get_floxmeta(&flox, "flox")
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

    #[tokio::test]
    async fn test_create_env() {
        let (flox, _tempdir_handle) = flox_instance();

        let meta_repo = flox.cache_dir.join(FLOXMETA_DIR_NAME).join("test");
        tokio::fs::create_dir_all(&meta_repo).await.unwrap();

        let _git = GitCommandProvider::init(&meta_repo, true).await.unwrap();

        let floxmeta = Floxmeta::<GitCommandProvider, ReadOnly<_>>::get_floxmeta(&flox, "test")
            .await
            .expect("Should open floxmeta repo");

        let floxmeta = floxmeta
            .enter_transaction()
            .await
            .expect("Should enter transaction");
        floxmeta
            .create_environment("xyz")
            .await
            .expect("Should create environment");

        let floxmeta = floxmeta
            .commit_transaction("Create environment")
            .await
            .expect("Should commit transaction");

        let environments = floxmeta
            .environments()
            .await
            .expect("Should find environments");
        assert!(!environments.is_empty());

        floxmeta
            .environment("xyz")
            .await
            .expect("Should find 'xyz' environment");

        let floxmeta = floxmeta
            .enter_transaction()
            .await
            .expect("Should enter transaction");

        assert!(matches!(
            floxmeta.create_environment("xyz").await,
            Err(CreateEnvironmentError::EnvironmentExists(_, _))
        ));

        let _ = floxmeta.abort_transaction().await;
    }
}
