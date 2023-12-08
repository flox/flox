use std::path::PathBuf;

use flox_types::version::Version;
use futures::{StreamExt, TryStreamExt};
use log::warn;
use runix::command::FlakeInit;
use runix::{NixBackend, Run};
use tempfile::TempDir;
use thiserror::Error;
use tokio::fs;

pub mod user_meta;

use self::user_meta::{SetUserMetaError, FLOX_USER_META_FILE};
use super::root::reference::ProjectDiscoverGitError;
use super::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use super::root::{Closed, Root};
use crate::flox::Flox;
use crate::models::floxmeta::user_meta::UserMeta;
use crate::providers::git::{GitCommandError, GitCommandProvider as Git, GitProvider};

pub const FLOXMETA_DIR_NAME: &str = "meta";

#[derive(Debug)]
pub struct Floxmeta<'flox, Access> {
    pub(crate) flox: &'flox Flox,
    pub(crate) owner: String,

    pub(crate) access: Access,
}

impl<'flox> Root<'flox, Closed<Git>> {
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
    pub fn guard_floxmeta(self) -> Result<Floxmeta<'flox, ReadOnly>, OpenFloxmetaError> {
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
        })
    }
}

/// Constructors and implementations for retrieving floxmeta handles
/// and creating a writable transaction
impl<'flox> Floxmeta<'flox, ReadOnly> {
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
    ) -> Result<Floxmeta<'flox, ReadOnly>, CreateFloxmetaError> {
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
        let git = Git::init(&user_floxmeta_prepare_dir, true).map_err(CreateFloxmetaError::Init)?;

        let floxmeta_prepare = Floxmeta {
            owner: owner.to_string(),
            flox,
            access: ReadOnly::new(git),
        };

        let user_meta = UserMeta {
            channels: Default::default(),
            client_uuid: uuid::Uuid::new_v4(),
            metrics_consent: None,
            version: Version::<1>,
        };

        let floxmeta = floxmeta_prepare.enter_transaction()?;
        floxmeta
            .access
            .git()
            .rename_branch("floxmain")
            .map_err(CreateFloxmetaError::Rename)?;
        floxmeta.set_user_meta(&user_meta)?;
        let _ = floxmeta.commit_transaction(&format!("init: create {FLOX_USER_META_FILE}"))?;

        fs::create_dir_all(floxmeta_dir)
            .await
            .map_err(CreateFloxmetaError::CreateFloxmetaHome)?;
        fs::rename(user_floxmeta_prepare_dir, user_floxmeta_dir)
            .await
            .map_err(CreateFloxmetaError::MoveFloxmeta)?;

        let floxmeta = Self::get_floxmeta(flox, owner)?;

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
    ) -> Result<Vec<Floxmeta<'flox, ReadOnly>>, ListFloxmetaError> {
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
            let floxmeta = Self::get_floxmeta(flox, &owner)?;
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
    pub fn get_floxmeta(
        flox: &'flox Flox,
        owner: &str,
    ) -> Result<Floxmeta<'flox, ReadOnly>, GetFloxmetaError> {
        let floxmeta_dir = flox.cache_dir.join(FLOXMETA_DIR_NAME).join(owner);
        let git = flox
            .resource(floxmeta_dir.clone())
            .guard()
            .map_err(|e| GetFloxmetaError::DiscoverGitDir(floxmeta_dir, e))?
            .ensure(|_| Err(GetFloxmetaError::NotFound(owner.to_string())))?;

        let floxmeta = git.guard_floxmeta()?;

        Ok(floxmeta)
    }

    pub fn enter_transaction(self) -> Result<Floxmeta<'flox, GitSandBox>, TransactionEnterError> {
        let transaction_temp_dir =
            TempDir::new_in(&self.flox.temp_dir).map_err(TransactionEnterError::CreateTempdir)?;

        let transaction_git = <Git as GitProvider>::clone(
            self.access.git().path(),
            transaction_temp_dir.path(),
            false,
        )
        .map_err(TransactionEnterError::GitClone)?;

        let sandbox = self
            .access
            .to_sandbox_in(transaction_temp_dir, transaction_git);

        Ok(Floxmeta {
            owner: self.owner.clone(),
            flox: self.flox,
            access: sandbox,
        })
    }
}

/// Constructors and implementations for retrieving floxmeta handles
/// and creating a writable transaction
impl<'flox, Access: GitAccess> Floxmeta<'flox, Access> {
    pub fn fetch(&self) -> Result<(), FetchError> {
        self.git().fetch().map_err(FetchError)?;
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
impl<'flox> Floxmeta<'flox, GitSandBox> {
    /// abort a transaction by discarding the temporary clone
    pub fn abort_transaction(self) -> Floxmeta<'flox, ReadOnly> {
        let access = self.access.abort();
        Floxmeta {
            owner: self.owner,
            flox: self.flox,
            access,
        }
    }

    /// complete floxmeta transaction by committing staged changes and pushing to origin
    pub fn commit_transaction(
        self,
        message: &str,
    ) -> Result<Floxmeta<'flox, ReadOnly>, TransactionCommitError> {
        self.access
            .git()
            .commit(message)
            .map_err(TransactionCommitError::GitCommit)?;
        self.access
            .git()
            .push("origin", false)
            .map_err(TransactionCommitError::GitPush)?;

        Ok(Floxmeta {
            owner: self.owner,
            flox: self.flox,
            access: self.access.abort(),
        })
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
pub enum CreateFloxmetaError {
    #[error("A floxmeta '{0}' already exists")]
    Exists(String),
    #[error("Could not create floxmeta initial directory: {0}")]
    CreateInitialDir(std::io::Error),
    #[error("Could not create floxmeta home directory: {0}")]
    CreateFloxmetaHome(std::io::Error),
    #[error("Could not move floxmeta repository to floxmeta home: {0}")]
    MoveFloxmeta(std::io::Error),
    #[error("Could not initialize git repo: {0}")]
    Init(GitCommandError),
    #[error("Could not make repo writable: {0}")]
    Transacton(#[from] TransactionEnterError),
    #[error("Could not rename 'floxmain' branch: {0}")]
    Rename(GitCommandError),
    #[error("Could not write back user metadata: {0}")]
    UserMeta(#[from] SetUserMetaError),
    #[error("Could not write back initialized floxmeta")]
    Commit(#[from] TransactionCommitError),
    #[error("Could not read created floxmeta: {0}")]
    GetCreated(#[from] GetFloxmetaError),
}

#[derive(Error, Debug)]
pub enum InitFloxmetaError<Nix: NixBackend>
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
    GitAdd(GitCommandError),
}

#[derive(Error, Debug)]
pub enum ListFloxmetaError {
    #[error("Could not open floxmeta cache ({0:?}): ({1})")]
    OpenMetaDir(PathBuf, std::io::Error),

    #[error("Could not create floxmeta cache ({0:?}): ({1})")]
    CreateMetaDir(PathBuf, std::io::Error),

    #[error(transparent)]
    GetFloxmeta(#[from] GetFloxmetaError),
}

#[derive(Error, Debug)]
pub enum GetFloxmetaError {
    #[error("Error opening floxmeta dir for ({0:?}): ({1})")]
    DiscoverGitDir(PathBuf, ProjectDiscoverGitError),

    #[error("Error opening floxmeta dir for {0:?}: Not found")]
    NotFound(String),

    #[error(transparent)]
    OpenFloxmeta(#[from] OpenFloxmetaError),
}

#[derive(Error, Debug)]
pub enum CreateEnvironmentError {
    #[error("Environment '{0}' ({1}) already exists")]
    EnvironmentExists(String, String),

    #[error("Failed checking out branch '{0}': {1}")]
    GitCheckout(String, GitCommandError),

    #[error("Failed to open metadata file for writing: {0}")]
    OpenMetadataFile(std::io::Error),

    #[error("Failed serializing metadata: {0}")]
    SerializeMetadata(#[from] serde_json::Error),

    #[error("Failed writing metadata: {0}")]
    WriteMetadata(std::io::Error),

    #[error("Failed adding metadata to git index: {0}")]
    GitAdd(GitCommandError),
}

#[derive(Error, Debug)]
pub enum TransactionEnterError {
    #[error("Failed to create tempdir for transaction")]
    CreateTempdir(std::io::Error),
    #[error("Failed to clone env into tempdir")]
    GitClone(GitCommandError),
}
#[derive(Error, Debug)]
pub enum TransactionCommitError {
    #[error("Failed committing changes: {0}")]
    GitCommit(GitCommandError),
    #[error("Failed synchronizing changes: {0}")]
    GitPush(GitCommandError),
}

#[derive(Error, Debug)]
#[error("Failed updating floxmeta: {0}")]
pub struct FetchError(GitCommandError);
