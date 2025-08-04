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
use super::{
    ConcreteEnvironment,
    ENV_DIR_NAME,
    EnvironmentError,
    LOCKFILE_FILENAME,
    copy_dir_recursive,
};
use crate::flox::{EnvironmentName, Flox};
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

        // update the generation metadata and return a copy for the caller
        let new_generation_metadata = {
            let Some(new_generation_metadata) = metadata.generations.get_mut(&generation) else {
                return Err(GenerationsError::GenerationNotFound(*generation));
            };
            new_generation_metadata.last_active = Some(Utc::now());

            new_generation_metadata.clone()
        };

        metadata.current_gen = Some(generation);

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

    fn switch_generation(
        &mut self,
        flox: &Flox,
        generation: GenerationId,
    ) -> Result<(), EnvironmentError>;
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
    #[serde(default, skip)]
    pub history: History,

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

#[derive(Debug, Clone)]
pub struct AddGenerationOptions {
    pub author: String,
    pub hostname: String,
    pub timestamp: DateTime<Utc>,
    pub kind: HistoryKind,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct SwitchGenerationOptions {
    pub author: String,
    pub hostname: String,
    pub timestamp: DateTime<Utc>,
    pub summary: String,
    pub next_generation: GenerationId,
}

impl AllGenerationsMetadata {
    /// Add metadata for a new generation, as well as consistent history.
    /// The return provides the [GenerationId] of the added generation metadata,
    /// that should **subsequently** be used
    /// to add the associated generation files.
    pub fn add_generation(
        &mut self,
        AddGenerationOptions {
            author,
            hostname,
            timestamp,
            kind,
            summary,
        }: AddGenerationOptions,
    ) -> (GenerationId, &SingleGenerationMetadata, &HistorySpec) {
        // prepare new values

        // Returns the highest numbered generation so we know which number to assign
        // the new one. This protects against potentially overwriting another
        // generation if you're currently on e.g. 2, but the latest is 5.
        //
        // Keys should all be numbers, but if they aren't we provide a default value.
        let next_generation =
            GenerationId(*self.generations.keys().cloned().max().unwrap_or_default() + 1);
        let current_generation = self.current_gen;

        let generation_metadata = SingleGenerationMetadata {
            created: timestamp,
            // TODO: I think we allowed this to be empty, i.e. create generations without activating them,
            // but as far as I know we never wrote `None`.
            last_active: Some(timestamp),
            description: summary.clone(),
        };

        let history_spec = HistorySpec {
            author,
            hostname,
            timestamp,
            kind,
            summary,
            previous_generation: current_generation,
            current_generation: next_generation,
        };

        // update self
        self.generations
            .insert(next_generation, generation_metadata);
        self.current_gen = Some(next_generation);
        self.history.0.push(history_spec);

        let generation_metadata_ref = self
            .generations
            .get(&next_generation)
            .expect("generation should have been inserted");

        let history_ref = self
            .history
            .0
            .iter()
            .next_back()
            .expect("history event should have been inserted");

        (next_generation, generation_metadata_ref, history_ref)
    }

    /// Switch the active marked generation to `next_generation`.
    /// `next_generation` must exist, and must be different from the current generation.
    /// To switch, this methods will (1) update [Self::current_gen] to `next_generation`,
    /// (2) set the [SingleGenerationMetadata::last_active] timestamp of the `next_generation`,
    /// and record a history item of type [HistoryKind::SwitchGeneration].
    pub fn switch_generation(
        &mut self,
        SwitchGenerationOptions {
            author,
            hostname,
            timestamp,
            summary,
            next_generation,
        }: SwitchGenerationOptions,
    ) -> Result<(), GenerationsError> {
        let Some(previous_generation) = self.current_gen else {
            unreachable!("current generation is only unavailable before any generation was added")
        };

        if next_generation == previous_generation {
            return Err(GenerationsError::RollbackToCurrentGeneration);
        }

        // get the metadata to the switched to generation
        let Some(next_generation_metadata) = self.generations.get_mut(&next_generation) else {
            return Err(GenerationsError::GenerationNotFound(*next_generation));
        };

        let history_spec = HistorySpec {
            author,
            hostname,
            timestamp,
            previous_generation: Some(previous_generation),
            current_generation: next_generation,
            kind: HistoryKind::SwitchGeneration,
            _compat: Default::default(),
        };

        // update current active gen
        self.current_gen = Some(next_generation);

        // update the generation metadata
        next_generation_metadata.last_active = Some(timestamp);

        // add action to history
        self.history.0.push(HistorySpec {
            author,
            hostname,
            timestamp,
            previous_generation,
            current_generation: next_generation,
            kind: HistoryKind::SwitchGeneration,
            summary,
        });

        Ok(())
    }

    /// Create a new object from its parts,
    /// used in tests to create mocks.
    #[cfg(feature = "tests")]
    pub fn new(
        current_gen: GenerationId,
        generations: impl IntoIterator<Item = (GenerationId, SingleGenerationMetadata)>,
    ) -> Self {
        AllGenerationsMetadata {
            history: History::default(),
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
    Copy,
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

/// The type of history event that is associated with a change.
/// These are generation _creating_ changes (such as install, edit, etc.)
/// and metadata only changes such as switching generations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum HistoryKind {
    Install,
    Edit,
    Uninstall,
    Upgrade,

    IncludeUpgrade,

    SwitchGeneration,
    Other(String),
}

/// The structure of a single change, tying together
/// _who_ performed _what_ change, where and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySpec {
    // change provided
    /// Type of the change
    kind: HistoryKind,
    /// Producer generated summary of the change
    summary: String,

    // system provided
    /// Local username of the user performing the change
    author: String,
    /// Hostname of the machine, on which the change was made
    hostname: String,
    /// Timestamp associated with the change
    // for consistency with the existing SingleGenerationMetadata
    #[serde(with = "chrono::serde::ts_seconds")]
    timestamp: DateTime<Utc>,

    // associated generation(s)
    /// Currently active generation, e.g. created by the change
    /// or switched to.
    current_generation: GenerationId,
    /// Previous generation before a new generation was created,
    /// or the generation active before a generation switch.
    previous_generation: Option<GenerationId>,

    /// Additional unsupported fields.
    #[serde(flatten)]
    _compat: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, derive_more::AsRef)]
pub struct History(Vec<HistorySpec>);

impl<'h> IntoIterator for &'h History {
    type IntoIter = std::slice::Iter<'h, HistorySpec>;
    type Item = &'h HistorySpec;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl IntoIterator for History {
    type IntoIter = std::vec::IntoIter<HistorySpec>;
    type Item = HistorySpec;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl History {}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::Environment;
    use crate::models::environment::path_environment::test_helpers::new_path_environment;

    const GEN_ID_1: GenerationId = GenerationId(1);
    const GEN_ID_2: GenerationId = GenerationId(2);

    const AUTHOR: &str = "author";
    const HOSTNAME: &str = "host";

    fn default_add_generation_options() -> AddGenerationOptions {
        AddGenerationOptions {
            author: AUTHOR.into(),
            hostname: HOSTNAME.into(),
            timestamp: Utc::now(),
            kind: HistoryKind::Other("mock".into()),
            summary: "mock generation".into(),
        }
    }

    fn default_switch_generation_options(next_generation: GenerationId) -> SwitchGenerationOptions {
        SwitchGenerationOptions {
            author: AUTHOR.into(),
            hostname: HOSTNAME.into(),
            timestamp: Utc::now(),
            summary: "switch mock".into(),
            next_generation,
        }
    }

    mod metadata {

        use pretty_assertions::assert_eq;

        use super::default_switch_generation_options;
        use crate::models::environment::generations::tests::default_add_generation_options;
        use crate::models::environment::generations::{
            AllGenerationsMetadata,
            GenerationId,
            GenerationsError,
            HistoryKind,
            SwitchGenerationOptions,
        };

        /// Adding a generation adds consisten metadata, ie.
        ///
        /// * adds new [SingleGenerationMetadata]
        /// * updates the current generation
        /// * adds a history entry for adding the generation
        #[test]
        fn add_generation_adds_metadata_and_history() {
            let mut metadata = AllGenerationsMetadata::default();

            let options = default_add_generation_options();

            let (generation, generation_metadata, history) =
                metadata.add_generation(options.clone());

            let (generation_metadata, history) = (generation_metadata.clone(), history.clone());

            assert_eq!(metadata.current_gen, Some(generation));
            assert_eq!(generation_metadata.created, options.timestamp);
            assert_eq!(generation_metadata.last_active, Some(options.timestamp));
            assert_eq!(generation_metadata.description, options.summary);

            assert_eq!(history.author, options.author);
            assert_eq!(history.hostname, options.hostname);
            assert_eq!(history.current_generation, generation);
            assert_eq!(history.previous_generation, None);
            assert_eq!(history.kind, options.kind);
            assert_eq!(history.summary, options.summary);
            assert_eq!(history.timestamp, options.timestamp);
        }

        #[test]
        fn generation_counter_is_correctly_increased() {
            let mut metadata = AllGenerationsMetadata::default();
            let (first_generation, _, _) =
                metadata.add_generation(default_add_generation_options());

            let (second_generation, _, _) =
                metadata.add_generation(default_add_generation_options());

            assert_eq!(first_generation, GenerationId(1));
            assert_eq!(second_generation, GenerationId(2));

            metadata
                .switch_generation(default_switch_generation_options(first_generation))
                .unwrap();
            assert_eq!(metadata.current_gen, Some(first_generation));

            let (third_generation, _, _) =
                metadata.add_generation(default_add_generation_options());

            // generation counter continues at the current max (N=2) + 1
            assert_eq!(third_generation, GenerationId(3));
        }

        /// Switching generations
        ///
        /// * updates the current generation
        /// * updares the "last_active" timestamp of the switched to generation
        /// * adds a history entry for the switch
        #[test]
        fn switch_generation_updates_metadata() {
            fn assert_switched_state(
                metadata: &AllGenerationsMetadata,
                switch_generation_options: &SwitchGenerationOptions,
                generation_switched_from: GenerationId,
                generation_switched_to: GenerationId,
            ) {
                assert_eq!(
                    metadata.current_gen,
                    Some(generation_switched_to),
                    "current gen was not updated"
                );
                assert_eq!(
                    metadata.generations[&generation_switched_to].last_active,
                    Some(switch_generation_options.timestamp),
                    "timestamp was not updated"
                );

                let history_entry = metadata.history.0.last().unwrap();
                assert_eq!(history_entry.author, switch_generation_options.author);
                assert_eq!(history_entry.hostname, switch_generation_options.hostname);
                assert_eq!(history_entry.kind, HistoryKind::SwitchGeneration);
                assert_eq!(
                    history_entry.previous_generation,
                    Some(generation_switched_from)
                );
                assert_eq!(history_entry.current_generation, generation_switched_to);
                assert_eq!(history_entry.summary, switch_generation_options.summary);
                assert_eq!(history_entry.timestamp, switch_generation_options.timestamp);
            }

            let mut metadata = AllGenerationsMetadata::default();

            let first_generation_options = default_add_generation_options();
            let second_generation_options = default_add_generation_options();

            let (first_gen_id, ..) = metadata.add_generation(first_generation_options.clone());
            let (second_gen_id, ..) = metadata.add_generation(second_generation_options.clone());

            let switch_generation_options = default_switch_generation_options(first_gen_id);
            metadata
                .switch_generation(switch_generation_options.clone())
                .unwrap();
            assert_switched_state(
                &metadata,
                &switch_generation_options,
                second_gen_id,
                first_gen_id,
            );

            // switch back (roll forward)
            let switch_generation_options = default_switch_generation_options(second_gen_id);
            metadata
                .switch_generation(switch_generation_options.clone())
                .unwrap();
            assert_switched_state(
                &metadata,
                &switch_generation_options,
                first_gen_id,
                second_gen_id,
            );
        }

        #[test]
        fn switch_generation_does_not_allow_current_generation() {
            let mut metadata = AllGenerationsMetadata::default();
            let (generation_id, ..) = metadata.add_generation(default_add_generation_options());

            let result =
                metadata.switch_generation(default_switch_generation_options(generation_id));

            assert!(
                matches!(result, Err(GenerationsError::RollbackToCurrentGeneration)),
                "unexpected result {:?}",
                result
            )
        }
    }

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
