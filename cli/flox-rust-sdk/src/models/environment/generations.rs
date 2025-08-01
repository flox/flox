//! A representation of the generations of an environment
//!
//! Example File layout:
//!
//! ```ignore
//! ./
//! ├── 1
//! │  └── env
//! │     ├── manifest.toml
//! │     └── manifest.lock
//! ├── 2
//! │  └── env
//! │    └── manifest.toml (lockfile is optional)
//! ├── ... N
//! │  └── env
//! │     └── manifest.toml
//! └── metadata.json
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::{env, fs};

use chrono::{DateTime, Utc};
use enum_dispatch::enum_dispatch;
use flox_core::Version;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use thiserror::Error;

use super::core_environment::CoreEnvironment;
use super::fetcher::IncludeFetcher;
use super::managed_environment::ManagedEnvironment;
use super::remote_environment::RemoteEnvironment;
use super::{ConcreteEnvironment, ENV_DIR_NAME, LOCKFILE_FILENAME, copy_dir_recursive};
use crate::flox::EnvironmentName;
use crate::models::environment::{MANIFEST_FILENAME, UninitializedEnvironment};
use crate::providers::git::{
    GitCommandError,
    GitCommandOptions,
    GitCommandProvider,
    GitProvider,
    GitRemoteCommandError,
};

const GENERATIONS_METADATA_FILE: &str = "metadata.json";

/// Generations as a branch in a (bare) git repository.
/// In this state files are read only using `git show`.
/// Making commits to bare repositories is tricky and non-idiomatic,
/// thus writing commands are implemented on checked out branches.
///
/// Todo: rename to `Opaque`, `Bare`, ...?
///
/// See also: [ReadWrite]
pub struct ReadOnly {}

/// Generations as a checked out branch of a git clone of a [ReadOnly] repo.
/// In this state files are read and writeable using the filesystem.
///
/// Mutating commands are committed and pushed to the [ReadOnly] branch.
///
/// Instances of this type are created using [Generations::writable].
///
/// Todo: rename to `CheckedOut`, `Filesystem`, ...?
///
/// /// See also: [ReadOnly]
pub struct ReadWrite<'a> {
    /// A reference to the [ReadOnly] instance that will be released
    /// when this instance is dropped.
    _read_only: &'a mut Generations<ReadOnly>,
}

/// A representation of the generations of an environment
///
/// Essentially the branches in a Floxmeta repository.
///
/// Todo: merge with or integrate Floxmeta?
pub struct Generations<State = ReadOnly> {
    /// A floxmeta repository/branch that contains the generations of an environment
    ///
    /// - When [ReadOnly], this is assumed to be bare, but it is not enforced.
    ///   If not bare, updating it using `git push` may fail to update checked out branches.
    /// - When [ReadWrite], this is required to not be bare.
    ///   This is enforced when created using [Self::writable].
    repo: GitCommandProvider,

    /// The name of the branch containing the generations
    /// Used to pick the correct branch when showing files, cloning, and pushing.
    branch: String,

    /// The state of the generations view
    ///
    /// Should remain private to enforce the invariant that [ReadWrite]
    /// always refers to a cloned branch of a [ReadOnly] instance.
    _state: State,
}

impl<S> Generations<S> {
    /// Read the generations metadata for an environment
    pub fn metadata(&self) -> Result<AllGenerationsMetadata, GenerationsError> {
        read_metadata(&self.repo, &self.branch)
    }

    /// Read the manifest of a given generation and return its contents as a string
    pub fn manifest(&self, generation: usize) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        if !metadata.generations.contains_key(&generation.into()) {
            return Err(GenerationsError::GenerationNotFound(generation));
        }
        let manifest_osstr = self
            .repo
            .show(&format!(
                "{}:{}/{}/{}",
                self.branch, generation, ENV_DIR_NAME, MANIFEST_FILENAME
            ))
            .map_err(GenerationsError::ShowManifest)?;

        Ok(manifest_osstr.to_string_lossy().to_string())
    }

    /// Read the lockfile of a given generation and return its contents as a string.
    pub fn lockfile(&self, generation: usize) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        if !metadata.generations.contains_key(&generation.into()) {
            return Err(GenerationsError::GenerationNotFound(generation));
        }
        let lockfile_osstr = self
            .repo
            .show(&format!(
                "{}:{}/{}/{}",
                self.branch, generation, ENV_DIR_NAME, LOCKFILE_FILENAME
            ))
            .map_err(GenerationsError::ShowLockfile)?;

        Ok(lockfile_osstr.to_string_lossy().to_string())
    }

    /// Read the manifest of the current generation and return its contents as a string
    pub fn current_gen_manifest(&self) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;

        self.manifest(*current_gen)
    }

    /// Read the lockfile of the current generation and return its contents as a string.
    pub fn current_gen_lockfile(&self) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;

        self.lockfile(*current_gen)
    }

    pub(super) fn git(&self) -> &GitCommandProvider {
        &self.repo
    }
}

impl Generations<ReadOnly> {
    /// Create a new generations instance
    pub fn new(repo: GitCommandProvider, branch: String) -> Self {
        Self {
            repo,
            branch,
            _state: ReadOnly {},
        }
    }

    /// Initialize a new generations branch for an environment
    /// in an assumed empty branch.
    ///
    /// This will create a new (initial) commit with an initial metadata file.
    pub fn init(
        options: GitCommandOptions,
        checkedout_tempdir: impl AsRef<Path>,
        bare_tempdir: impl AsRef<Path>,
        branch: String,
        name: &EnvironmentName,
    ) -> Result<Self, GenerationsError> {
        let repo = GitCommandProvider::init_with(options.clone(), &checkedout_tempdir, false)
            .map_err(GenerationsError::InitRepo)?;
        repo.checkout(&branch, true)
            .map_err(GenerationsError::CreateBranch)?;

        let metadata = AllGenerationsMetadata::default();
        write_metadata_file(metadata, repo.path())?;

        repo.add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .map_err(GenerationsError::StageChanges)?;
        repo.commit(&format!(
            "Initialize generations branch for environment '{}'",
            name
        ))
        .map_err(GenerationsError::CommitChanges)?;

        let bare = GitCommandProvider::clone_branch_with(
            options,
            checkedout_tempdir.as_ref(),
            bare_tempdir,
            &branch,
            true,
        )
        .map_err(GenerationsError::MakeBareClone)?;

        Ok(Self::new(bare, branch))
    }

    /// Create a writable copy of this generations instance
    /// in a temporary directory.
    pub fn writable(
        &mut self,
        tempdir: impl AsRef<Path>,
    ) -> Result<Generations<ReadWrite<'_>>, GenerationsError> {
        if env::var("_FLOX_TESTING_NO_WRITABLE").is_ok() {
            panic!("Can't create writable generations when _FLOX_TESTING_NO_WRITABLE is set");
        }

        let repo = checkout_to_tempdir(
            &self.repo,
            &self.branch,
            tempfile::tempdir_in(tempdir).unwrap().keep(),
        )?;

        Ok(Generations {
            repo,
            branch: self.branch.clone(),
            _state: ReadWrite { _read_only: self },
        })
    }
}

impl Generations<ReadWrite<'_>> {
    /// Return a mutable [CoreEnvironment] instance for a given generation
    /// contained in the generations branch.
    ///
    /// Note:
    ///   Only the generations branch is isolated in a tempdir.
    ///   Taking a mutable reference to a generation will not isolate it further,
    ///   so changes to the generation will remain in the tempdir.
    ///   If [Generations::add_generation] is given a [CoreEnvironment] instance
    ///   returned by this method, it will copy the environment into the new generation.
    ///
    ///   When a generation needs to be used again after being modified,
    ///   it is recommended to create a new [Generations<ReadWrite>] instance first.
    pub fn get_generation(
        &self,
        generation: usize,
        include_fetcher: IncludeFetcher,
    ) -> Result<CoreEnvironment, GenerationsError> {
        let environment = CoreEnvironment::new(
            self.repo
                .path()
                .join(generation.to_string())
                .join(ENV_DIR_NAME),
            include_fetcher,
        );

        Ok(environment)
    }

    /// Return the current generation as set in the metadata file
    /// as a [CoreEnvironment].
    ///
    /// The generation can then be safely modified
    /// and registered as a new generation using [Self::add_generation].
    pub fn get_current_generation(
        &self,
        include_fetcher: IncludeFetcher,
    ) -> Result<CoreEnvironment, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;
        self.get_generation(*current_gen, include_fetcher)
    }

    /// Import an existing environment into a generation
    ///
    /// Assumes the invariant that the [CoreEnvironment] instance is valid.
    ///
    /// This will copy the manifest and lockfile from the source environment
    /// into a generation folder.
    /// Any other assets such as hook scripts are ignored.
    ///
    /// If the generation already exists, it will be overwritten.
    ///
    /// If `set_current` is true, the generation will also be set as the current generation.
    fn register_generation(
        &mut self,
        environment: &mut CoreEnvironment,
        generation: usize,
        description: String,
        set_current: bool,
    ) -> Result<(), GenerationsError> {
        let mut generation_metadata = SingleGenerationMetadata::new(description.clone());

        let mut metadata = self.metadata()?;

        if set_current {
            metadata.current_gen = Some(generation.into());
            generation_metadata.last_active = Some(Utc::now());
        }

        // Insert the new generation
        let _existing = metadata
            .generations
            .insert(generation.into(), generation_metadata);

        // Write the metadata file with the new generation added
        write_metadata_file(metadata, self.repo.path())?;

        let generation_path = self.repo.path().join(generation.to_string());
        let env_path = generation_path.join(ENV_DIR_NAME);
        fs::create_dir_all(&env_path).unwrap();

        // copy `env/`, i.e. manifest and lockfile (if it exists) and possibly other assets
        // copy into `<generation>/env/` to make creating `PathEnvironment` easier
        copy_dir_recursive(environment.path(), &env_path, true).unwrap();

        self.repo
            .add(&[&generation_path])
            .map_err(GenerationsError::StageChanges)?;
        self.repo
            .add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .map_err(GenerationsError::StageChanges)?;

        self.repo
            .commit(&format!(
                "Create generation {}\n\n{}",
                generation, description
            ))
            .map_err(GenerationsError::CommitChanges)?;
        self.repo
            .push("origin", false)
            .map_err(GenerationsError::CompleteTransaction)?;

        Ok(())
    }

    /// Create a new generation from an existing environment
    ///
    /// Assumes the invariant that the [PathEnvironment] instance is valid.
    ///
    /// This will copy the manifest and lockfile from the source environment
    /// into a generation folder.
    /// Any other assets such as hook scripts are ignored.
    ///
    /// This method assigns a new sequential generation number
    /// and sets it as the current generation.
    pub fn add_generation(
        &mut self,
        environment: &mut CoreEnvironment,
        description: String,
    ) -> Result<(), GenerationsError> {
        // Returns the highest numbered generation so we know which number to assign
        // the new one. This protects against potentially overwriting another
        // generation if you're currently on e.g. 2, but the latest is 5.
        //
        // Keys should all be numbers, but if they aren't we provide a default value.
        let max = self
            .metadata()?
            .generations
            .keys()
            .cloned()
            .max()
            .unwrap_or_default();

        self.register_generation(environment, *max + 1, description, true)
    }

    /// Switch to a provided generation to either roll backwards or forwards.
    ///
    /// Fails if the generation does not exist or is already the current generation.
    pub fn set_current_generation(
        &mut self,
        generation: GenerationId,
    ) -> Result<(GenerationId, SingleGenerationMetadata), GenerationsError> {
        let mut metadata = self.metadata()?;

        if Some(&generation) == metadata.current_gen.as_ref() {
            return Err(GenerationsError::RollbackToCurrentGeneration);
        }

        let Some(new_generation_metadata) = metadata.generations.get(&generation).cloned() else {
            return Err(GenerationsError::GenerationNotFound(*generation));
        };

        metadata.current_gen = Some(generation.clone());
        write_metadata_file(metadata, self.repo.path())?;

        self.repo
            .add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .map_err(GenerationsError::StageChanges)?;
        self.repo
            .commit(&format!("Set current generation to {}", generation))
            .map_err(GenerationsError::CommitChanges)?;
        self.repo
            .push("origin", false)
            .map_err(GenerationsError::CompleteTransaction)?;

        Ok((generation, new_generation_metadata))
    }
}

#[derive(Debug, Error)]
pub enum GenerationsError {
    #[error(
        "Generations are only available for environments pushed to floxhub.\n\
        The environment {0} is a local only environment."
    )]
    UnsupportedEnvironment(String),

    // region: initialization errors
    #[error("could not initialize generations repo")]
    InitRepo(#[source] GitCommandError),
    #[error("could not create generations branch")]
    CreateBranch(#[source] GitCommandError),
    #[error("could not make bare clone of generations branch")]
    MakeBareClone(#[source] GitRemoteCommandError),

    // endregion

    // region: metadata errors
    #[error("could not serialize generations metadata")]
    SerializeMetadata(#[source] serde_json::Error),
    #[error("could not write generations metadata file")]
    WriteMetadata(#[source] std::io::Error),

    #[error("could not show generations metadata file")]
    ShowMetadata(#[source] GitCommandError),
    #[error("could not parse generations metadata")]
    DeserializeMetadata(#[source] serde_json::Error),
    // endregion

    // region: generation errors
    #[error("generation {0} not found")]
    GenerationNotFound(usize),
    #[error("no generations found in environment")]
    NoGenerations,
    #[error("cannot rollback to current generation")]
    RollbackToCurrentGeneration,
    // endregion

    // region: repo/transaction
    #[error("could not clone generations branch")]
    CloneToFS(#[source] GitRemoteCommandError),
    #[error("could not stage changes")]
    StageChanges(#[source] GitCommandError),
    #[error("could not commit changes")]
    CommitChanges(#[source] GitCommandError),
    #[error("could not complete transaction")]
    CompleteTransaction(#[source] GitRemoteCommandError),
    // endregion

    // region: manifest errors
    #[error("could not write manifest file")]
    WriteManifest(#[source] std::io::Error),
    #[error("could not show manifest file")]
    ShowManifest(#[source] GitCommandError),
    #[error("could not show lockfile")]
    ShowLockfile(#[source] GitCommandError),
    // endregion
}

/// Realize the generations branch into a temporary directory
fn checkout_to_tempdir(
    repo: &GitCommandProvider,
    branch: &str,
    tempdir: PathBuf,
) -> Result<GitCommandProvider, GenerationsError> {
    let git_options = repo.get_options().clone();
    let repo =
        GitCommandProvider::clone_branch_with(git_options, repo.path(), tempdir, branch, false)
            .map_err(GenerationsError::CloneToFS)?;

    Ok(repo)
}

/// Reads the generations metadata file directly from the repository
fn read_metadata(
    repo: &GitCommandProvider,
    ref_name: &str,
) -> Result<AllGenerationsMetadata, GenerationsError> {
    let metadata = {
        let metadata_content = repo
            .show(&format!("{}:{}", ref_name, GENERATIONS_METADATA_FILE))
            .map_err(GenerationsError::ShowMetadata)?;
        serde_json::from_str(&metadata_content.to_string_lossy())
            .map_err(GenerationsError::DeserializeMetadata)?
    };
    Ok(metadata)
}

/// Serializes the generations metadata file to a path
///
/// The path is expected to be a realized generations repository.
fn write_metadata_file(
    metadata: AllGenerationsMetadata,
    realized_path: &Path,
) -> Result<(), GenerationsError> {
    let metadata_content =
        serde_json::to_string(&metadata).map_err(GenerationsError::SerializeMetadata)?;
    let metadata_path = realized_path.join(GENERATIONS_METADATA_FILE);
    fs::write(metadata_path, metadata_content).map_err(GenerationsError::WriteMetadata)?;
    Ok(())
}

/// Generation related methods for environments that support generations.
/// In practice that's [ManagedEnvironment] and [RemoteEnvironment].
/// We use a cummon trait to ensure common and consistent functionality
/// and allow static dispatch from [GenerationsEnvironment]
/// to the concrete implementations.
#[enum_dispatch]
pub trait GenerationsExt {
    /// Return all generations metadata for the environment.
    fn generations_metadata(&self) -> Result<AllGenerationsMetadata, GenerationsError>;
}

/// Combined type for environments supporting generations,
/// i.e. local or remote managed environemnts.
/// We use this in addition to the [GenerationsExt] trait,
/// to avoid forcing `dyn compatibility` on [GenerationsExt],
/// and repeated deconstruction of [ConcreteEnvironment]s,
/// similarly to how/why we wrap all [Environment] implementations
/// under [ConcreteEnvironment].
///
/// To be created either via [ConcreteEnvironment::try_into],
/// or the [Into] implementations for the subjects.
#[derive(Debug)]
#[enum_dispatch(GenerationsExt, Environment)]
pub enum GenerationsEnvironment {
    Managed(ManagedEnvironment),
    Remote(RemoteEnvironment),
}

impl TryFrom<ConcreteEnvironment> for GenerationsEnvironment {
    type Error = GenerationsError;

    fn try_from(env: ConcreteEnvironment) -> std::result::Result<Self, Self::Error> {
        let env = match env {
            ConcreteEnvironment::Path(_) => {
                let description =
                    UninitializedEnvironment::from_concrete_environment(&env).bare_description();
                return Err(GenerationsError::UnsupportedEnvironment(description));
            },
            ConcreteEnvironment::Managed(env) => env.into(),
            ConcreteEnvironment::Remote(env) => env.into(),
        };

        Ok(env)
    }
}

/// flox environment metadata for managed environments
///
/// Managed environments support rolling back to previous generations.
/// Generations are defined as immutable copy-on-write folders.
/// Rollbacks and associated [SingleGenerationMetadata] are tracked per environment
/// in a metadata file at the root of the environment branch.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct AllGenerationsMetadata {
    /// None means the environment has been created but does not yet have any
    /// generations
    pub current_gen: Option<GenerationId>,
    /// Metadata for all generations of the environment.
    /// Entries in this map must match up 1-to-1 with the generation folders
    /// in the environment branch.
    pub generations: BTreeMap<GenerationId, SingleGenerationMetadata>,
    /// Schema version of the metadata file, not yet utilized
    #[serde(default)]
    version: Version<1>,
}

impl AllGenerationsMetadata {
    #[cfg(feature = "tests")]
    pub fn new(
        current_gen: GenerationId,
        generations: impl IntoIterator<Item = (GenerationId, SingleGenerationMetadata)>,
    ) -> Self {
        AllGenerationsMetadata {
            current_gen: Some(current_gen),
            generations: BTreeMap::from_iter(generations),
            version: Default::default(),
        }
    }
}

/// Metadata for a single generation of an environment
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SingleGenerationMetadata {
    /// unix timestamp of the creation time of this generation
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created: DateTime<Utc>,

    /// unix timestamp of the time when this generation was last set as active
    /// `None` if this generation has never been set as active
    #[serde(with = "chrono::serde::ts_seconds_option")]
    pub last_active: Option<DateTime<Utc>>,

    /// log message(s) describing the change from the previous generation
    pub description: String,
    // todo: do we still need to track this?
    //       do we now?
    // /// store path of the built generation
    // path: PathBuf,
}

impl SingleGenerationMetadata {
    /// Create a new generation metadata instance
    pub fn new(description: String) -> Self {
        Self {
            created: Utc::now(),
            last_active: None,
            description,
        }
    }
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Default,
    derive_more::Deref,
    derive_more::DerefMut,
    derive_more::From,
    derive_more::Display,
    derive_more::FromStr,
    DeserializeFromStr,
    SerializeDisplay,
)]
pub struct GenerationId(usize);

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::Environment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment;

    const GEN_ID_1: GenerationId = GenerationId(1);
    const GEN_ID_2: GenerationId = GenerationId(2);

    fn setup_two_generations() -> (Generations, TempDir) {
        let (flox, tempdir) = flox_instance();
        let env = new_path_environment(&flox, "version = 1");
        let env_name = env.name();
        let mut core_env = env.into_core_environment().unwrap();

        let floxmeta_checkedout_path = tempfile::tempdir_in(&tempdir).unwrap().keep();
        let floxmeta_temp_path = tempfile::tempdir_in(&tempdir).unwrap().keep();

        let mut generations = Generations::init(
            GitCommandOptions::default(),
            floxmeta_checkedout_path,
            floxmeta_temp_path,
            "some-branch".to_string(),
            &env_name,
        )
        .unwrap();

        let mut generations_rw = generations.writable(&tempdir).unwrap();
        generations_rw
            .add_generation(&mut core_env, "First generation".to_string())
            .unwrap();
        generations_rw
            .add_generation(&mut core_env, "Second generation".to_string())
            .unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen,
            Some(GEN_ID_2),
            "should be at second generation"
        );

        (generations, tempdir)
    }

    #[test]
    fn set_current_generation_backwards_and_forwards() {
        let (mut generations, tempdir) = setup_two_generations();
        let mut generations_rw = generations.writable(&tempdir).unwrap();

        assert_eq!(
            generations_rw.metadata().unwrap().current_gen,
            Some(GEN_ID_2),
            "should start at second generation"
        );

        generations_rw.set_current_generation(GEN_ID_1).unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen,
            Some(GEN_ID_1),
            "should roll back to first generation"
        );

        generations_rw.set_current_generation(GEN_ID_2).unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen,
            Some(GEN_ID_2),
            "should roll forwards to second generation"
        );
    }

    #[test]
    fn set_current_generation_not_found() {
        let (mut generations, tempdir) = setup_two_generations();
        let mut generations_rw = generations.writable(&tempdir).unwrap();

        let res = generations_rw.set_current_generation(GenerationId(10));
        assert!(
            matches!(res, Err(GenerationsError::GenerationNotFound(10))),
            "should error when setting nonexistent generation, got: {:?}",
            res
        );
    }

    #[test]
    fn set_current_generation_already_current() {
        let (mut generations, tempdir) = setup_two_generations();
        let mut generations_rw = generations.writable(&tempdir).unwrap();

        let current_gen = generations_rw.metadata().unwrap().current_gen.unwrap();
        let res = generations_rw.set_current_generation(current_gen);
        assert!(
            matches!(res, Err(GenerationsError::RollbackToCurrentGeneration)),
            "should error when setting current generation, got: {:?}",
            res
        );
    }
}
