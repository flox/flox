use std::marker::PhantomData;
use std::path::{Path, PathBuf};

use flox_types::version::Version;
use futures::{StreamExt, TryStreamExt};
use log::warn;
use runix::command::FlakeInit;
use runix::{NixBackend, Run};
use tempfile::TempDir;
use thiserror::Error;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

mod environment;
pub mod user_meta;
use environment::{Metadata, METADATA_JSON};

use self::user_meta::{SetUserMetaError, FLOX_USER_META_FILE};
use super::root::reference::ProjectDiscoverGitError;
use super::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use super::root::{Closed, Root};
use crate::flox::Flox;
use crate::models::floxmeta::user_meta::UserMeta;
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

impl<'flox> Root<'flox, Closed<String>> {}

impl<'flox, Git: GitProvider> Clone for Floxmeta<'flox, Git, ReadOnly<Git>> {
    fn clone(&self) -> Self {
        Self {
            flox: self.flox,
            owner: self.owner.clone(),
            access: self.access.read_only(),
            _git: self._git,
        }
    }
}

/// Constructors and implementations for retrieving floxmeta handles
/// and creating a writable transaction
impl<'flox, Git: GitProvider> Floxmeta<'flox, Git, ReadOnly<Git>> {
    /// Creates a new floxmeta for the specified owner
    ///
    /// ## Return value
    ///
    /// returns [Floxmeta] instance for the newly created floxmeta dir
    ///
    /// returns [CreateFloxmetaError]
    ///
    /// * if a floxmeta for the specified owner already exists.
    /// * if creating initializing a floxmeta in a tempdir fails at any step
    /// * if the temporary floxmeta cannot be moved to its final location
    pub async fn create_floxmeta(
        flox: &'flox Flox,
        owner: &str,
    ) -> Result<Floxmeta<'flox, Git, ReadOnly<Git>>, CreateFloxmetaError<Git>> {
        let floxmeta_dir = flox.cache_dir.join(FLOXMETA_DIR_NAME);
        let user_floxmeta_dir = flox.cache_dir.join(FLOXMETA_DIR_NAME).join(owner);
        let user_floxmeta_prepare_dir = flox.temp_dir.join(FLOXMETA_DIR_NAME).join(owner);

        // simple check if floxmeta already exists
        if user_floxmeta_dir.exists() {
            return Err(CreateFloxmetaError::Exists(owner.to_string()));
        }

        fs::create_dir_all(&user_floxmeta_prepare_dir)
            .await
            .map_err(CreateFloxmetaError::CreateInitialDir)?;

        // We are creating the floxmeta in a tempdir.
        // After finishing the initialization is complete and successful
        // the floxmeta is moved to its final place
        // TODO use --initial-branch instead of renaming to floxmeta
        let git = Git::init(&user_floxmeta_prepare_dir, true)
            .await
            .map_err(CreateFloxmetaError::Init)?;

        let floxmeta_prepare = Floxmeta {
            owner: owner.to_string(),
            flox,
            access: ReadOnly::new(git),
            _git: PhantomData::default(),
        };

        let user_meta = UserMeta {
            channels: Default::default(),
            client_uuid: uuid::Uuid::new_v4(),
            metrics_consent: None,
            version: Version::<1>,
        };

        let floxmeta = floxmeta_prepare.enter_transaction().await?;
        floxmeta
            .access
            .git()
            .rename_branch("floxmain")
            .await
            .map_err(CreateFloxmetaError::Rename)?;
        floxmeta.set_user_meta(&user_meta).await?;
        let _ = floxmeta
            .commit_transaction(&format!("init: create {FLOX_USER_META_FILE}"))
            .await?;

        fs::create_dir_all(floxmeta_dir)
            .await
            .map_err(CreateFloxmetaError::CreateFloxmetaHome)?;
        fs::rename(user_floxmeta_prepare_dir, user_floxmeta_dir)
            .await
            .map_err(CreateFloxmetaError::MoveFloxmeta)?;

        let floxmeta = Self::get_floxmeta(flox, owner).await?;

        Ok(floxmeta)
    }

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

        futures::stream::iter(
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
        .await
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
    pub async fn fetch(&self) -> Result<(), FetchError<Git>> {
        self.git().fetch().await.map_err(FetchError)?;
        Ok(())
    }

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
        let orig_git_config_global = std::env::var("GIT_CONFIG_GLOBAL");
        std::env::set_var("GIT_CONFIG_GLOBAL", self.flox.config_dir.join("gitconfig"));
        let orig_git_config_system = std::env::var("GIT_CONFIG_SYSTEM");
        std::env::set_var("GIT_CONFIG_SYSTEM", "/dev/null");

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

        if let Ok(orig_git_config_global) = orig_git_config_global {
            std::env::set_var("GIT_CONFIG_GLOBAL", orig_git_config_global);
        }
        if let Ok(orig_git_config_system) = orig_git_config_system {
            std::env::set_var("GIT_CONFIG_SYSTEM", orig_git_config_system);
        }

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

/// Errors occurring while trying to upgrade to an [`Open<Git>`] [Root]
#[derive(Error, Debug)]
pub enum OpenFloxmetaError {
    #[error("Could not determine repository root")]
    WorkdirNotFound,
}

/// Errors occurring while trying to create a floxmeta
#[derive(Error, Debug)]
pub enum CreateFloxmetaError<Git: GitProvider> {
    #[error("A floxmeta '{0}' already exists")]
    Exists(String),
    #[error("Could not create floxmeta initial directory: {0}")]
    CreateInitialDir(std::io::Error),
    #[error("Could not create floxmeta home directory: {0}")]
    CreateFloxmetaHome(std::io::Error),
    #[error("Could not move floxmeta repository to floxmeta home: {0}")]
    MoveFloxmeta(std::io::Error),
    #[error("Could not initialize git repo: {0}")]
    Init(Git::InitError),
    #[error("Could not make repo writable: {0}")]
    Transacton(#[from] TransactionEnterError<Git>),
    #[error("Could not rename 'floxmain' branch: {0}")]
    Rename(Git::RenameError),
    #[error("Could not write back user metadata: {0}")]
    UserMeta(#[from] SetUserMetaError<Git>),
    #[error("Could not write back initialized floxmeta")]
    Commit(#[from] TransactionCommitError<Git>),
    #[error("Could not read created floxmeta: {0}")]
    GetCreated(#[from] GetFloxmetaError<Git>),
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
    #[error("Failed committing changes: {0}")]
    GitCommit(Git::CommitError),
    #[error("Failed synchronizing changes: {0}")]
    GitPush(Git::PushError),
}

#[derive(Error, Debug)]
#[error("Failed updating floxmeta: {0}")]
pub struct FetchError<Git: GitProvider>(Git::FetchError);

#[cfg(test)]
#[cfg(feature = "impure-unit-tests")]
pub(super) mod floxmeta_tests {
    use tempfile::TempDir;

    use super::*;
    use crate::providers::git::GitCommandProvider;

    pub(super) fn flox_instance() -> (Flox, TempDir) {
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

    #[tokio::test]
    async fn test_create_floxmeta() {
        let (flox, _tempdir_handle) = flox_instance();

        Floxmeta::<GitCommandProvider, ReadOnly<_>>::get_floxmeta(&flox, "someone")
            .await
            .expect_err("Should fail finding floxmeta");
        let floxmeta =
            Floxmeta::<GitCommandProvider, ReadOnly<_>>::create_floxmeta(&flox, "someone")
                .await
                .expect("should create a floxmeta");
        floxmeta.user_meta().await.expect("should find user_meta");

        Floxmeta::<GitCommandProvider, ReadOnly<_>>::create_floxmeta(&flox, "someone")
            .await
            .expect_err("should fail if floxmeta exists");
    }
}
