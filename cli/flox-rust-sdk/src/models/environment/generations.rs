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
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::{env, fs};

use chrono::{DateTime, Utc};
use enum_dispatch::enum_dispatch;
use flox_core::Version;
use itertools::Itertools;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use serde_with::{DeserializeFromStr, SerializeDisplay, skip_serializing_none};
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

    /// The author modifying the generations, usually the current $USER.
    author: String,
    /// The hostname of the machine on which generations are modified, usually $HOST
    hostname: String,

    argv: Vec<String>,
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
    pub fn metadata(&self) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
        read_metadata(&self.repo, &self.branch)
    }

    /// Read the manifest of a given generation and return its contents as a string
    pub fn manifest(&self, generation: usize) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        if !metadata.generations().contains_key(&generation.into()) {
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
        if !metadata.generations().contains_key(&generation.into()) {
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
            .current_gen()
            .ok_or(GenerationsError::NoGenerations)?;

        self.manifest(*current_gen)
    }

    /// Read the lockfile of the current generation and return its contents as a string.
    pub fn current_gen_lockfile(&self) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen()
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
        write_metadata_file(metadata.into(), repo.path())?;

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
        author: impl Into<String>,
        hostname: impl Into<String>,
        argv: &[String],
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
            _state: ReadWrite {
                _read_only: self,
                author: author.into(),
                hostname: hostname.into(),
                argv: argv.to_owned(),
            },
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
            .current_gen()
            .ok_or(GenerationsError::NoGenerations)?;
        self.get_generation(*current_gen, include_fetcher)
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
        change_kind: HistoryKind,
    ) -> Result<(), GenerationsError> {
        // add metadata
        // this returns a free generation id to store the env files under
        let mut metadata = self.metadata()?;
        let (generation, history_item) = metadata.add_generation(AddGenerationOptions {
            author: self._state.author.clone(),
            hostname: self._state.hostname.clone(),
            argv: self._state.argv.clone(),
            timestamp: Utc::now(),
            kind: change_kind,
        });

        let summary = history_item.summary();

        // Write the metadata file with the new generation added
        write_metadata_file(metadata, self.repo.path())?;

        // copy generation environment files
        let generation_path = self.repo.path().join(generation.to_string());
        let env_path = generation_path.join(ENV_DIR_NAME);
        fs::create_dir_all(&env_path).unwrap();

        // copy `env/`, i.e. manifest and lockfile (if it exists) and possibly other assets
        // copy into `<generation>/env/` to make creating `PathEnvironment` easier
        copy_dir_recursive(environment.path(), &env_path, true).unwrap();

        // commit environment and metadata
        self.repo
            .add(&[&generation_path])
            .map_err(GenerationsError::StageChanges)?;
        self.repo
            .add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .map_err(GenerationsError::StageChanges)?;

        self.repo
            .commit(&format!("Create generation {}\n\n{}", generation, summary))
            .map_err(GenerationsError::CommitChanges)?;
        self.repo
            .push("origin", false)
            .map_err(GenerationsError::CompleteTransaction)?;

        Ok(())
    }

    /// Switch to a provided generation to either roll backwards or forwards.
    ///
    /// Fails if the generation does not exist or is already the current generation.
    pub fn set_current_generation(
        &mut self,
        next_generation: GenerationId,
    ) -> Result<(), GenerationsError> {
        let mut metadata = self.metadata()?;

        let (_, history_item) = metadata.switch_generation(SwitchGenerationOptions {
            author: self._state.author.clone(),
            hostname: self._state.hostname.clone(),
            argv: self._state.argv.clone(),
            timestamp: Utc::now(),
            next_generation,
        })?;
        let summary = history_item.summary();

        write_metadata_file(metadata, self.repo.path())?;

        self.repo
            .add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .map_err(GenerationsError::StageChanges)?;
        self.repo
            .commit(&summary)
            .map_err(GenerationsError::CommitChanges)?;
        self.repo
            .push("origin", false)
            .map_err(GenerationsError::CompleteTransaction)?;

        Ok(())
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

    #[error("failed to migrate v1 metatada: {0}")]
    MigrateV1ToV2(String),

    #[error("could not show generations metadata file")]
    ShowMetadata(#[source] GitCommandError),
    #[error("could not parse generations metadata")]
    DeserializeMetadata(#[source] serde_json::Error),
    #[error("Environment metadata of version '{0}' could not be parsed into its expected schema.")]
    InvalidSchema(serde_json::Value),
    #[error(
        "Environment metadata of version '{0}' is not supported\n\
         \n\
         This environment appears to have been modified by a newer version of Flox.\n\
         Please upgrade to the latest version of Flox and try again."
    )]
    InvalidVersion(serde_json::Value),
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
) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
    let metadata_content = repo
        .show(&format!("{}:{}", ref_name, GENERATIONS_METADATA_FILE))
        .map_err(GenerationsError::ShowMetadata)?;

    parse_metadata(&mut serde_json::Deserializer::from_slice(
        metadata_content.as_bytes(),
    ))
}

fn parse_metadata<'de>(
    deserializer: impl Deserializer<'de, Error = serde_json::Error>,
) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError> {
    #[derive(Debug, Deserialize)]
    #[serde(untagged)]
    enum MetadataVersionCompat {
        V1(compat::AllGenerationsMetadataV1),
        V2(WithOtherFields<AllGenerationsMetadata>),
        VX { version: serde_json::Value },
    }

    let metadata: MetadataVersionCompat = MetadataVersionCompat::deserialize(deserializer)
        .map_err(GenerationsError::DeserializeMetadata)?;

    let metadata = match metadata {
        MetadataVersionCompat::V1(all_generations_metadata_v1) => {
            let migrated: AllGenerationsMetadata = all_generations_metadata_v1.try_into()?;
            migrated.into()
        },
        MetadataVersionCompat::V2(all_generations_metadata) => all_generations_metadata,
        MetadataVersionCompat::VX { version }
            if version == Value::Number(1.into()) || version == Value::Number(2.into()) =>
        {
            Err(GenerationsError::InvalidSchema(version))?
        },

        MetadataVersionCompat::VX { version } => Err(GenerationsError::InvalidVersion(version))?,
    };

    Ok(metadata)
}

/// Serializes the generations metadata file to a path
///
/// The path is expected to be a realized generations repository.
fn write_metadata_file(
    metadata: WithOtherFields<AllGenerationsMetadata>,
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
    fn generations_metadata(
        &self,
    ) -> Result<WithOtherFields<AllGenerationsMetadata>, GenerationsError>;

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
#[derive(Serialize, Deserialize, Debug, Default, PartialEq, JsonSchema)]
#[skip_serializing_none]
pub struct AllGenerationsMetadata {
    /// Schema version of the metadata file
    version: Version<2>,
    history: History,
    total_generations: usize,
}

#[derive(Debug, Clone)]
pub struct AddGenerationOptions {
    pub author: String,
    pub hostname: String,
    pub argv: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub kind: HistoryKind,
}

#[derive(Debug, Clone)]
pub struct SwitchGenerationOptions {
    pub author: String,
    pub hostname: String,
    pub argv: Vec<String>,
    pub timestamp: DateTime<Utc>,
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
            argv,
            timestamp,
            kind,
        }: AddGenerationOptions,
    ) -> (GenerationId, &HistorySpec) {
        // prepare new values

        // Returns the highest numbered generation so we know which number to assign
        // the new one. This protects against potentially overwriting another
        // generation if you're currently on e.g. 2, but the latest is 5.
        //
        // Keys should all be numbers, but if they aren't we provide a default value.
        let next_generation = GenerationId(self.total_generations + 1);
        let current_generation = self.current_gen();

        let history_spec = HistorySpec {
            author,
            hostname,
            command: Self::parse_argv(argv),
            timestamp,
            kind,
            previous_generation: current_generation,
            current_generation: next_generation,
        };

        // update self
        self.history.0.push(history_spec.into());
        self.total_generations = self.total_generations.saturating_add(1);

        let history_ref = self
            .history
            .0
            .iter()
            .next_back()
            .expect("history event should have been inserted");

        (next_generation, history_ref)
    }

    /// Switch the live generation to `next_generation`.
    /// `next_generation` must exist, and must be different from the current generation.
    /// To switch, this methods will (1) update [Self::current_gen] to `next_generation`,
    /// (2) set the [SingleGenerationMetadata::last_live] timestamp of the `next_generation`,
    /// and record a history item of type [HistoryKind::SwitchGeneration].
    pub fn switch_generation(
        &mut self,
        SwitchGenerationOptions {
            author,
            hostname,
            argv,
            timestamp,
            next_generation,
        }: SwitchGenerationOptions,
    ) -> Result<(GenerationId, &HistorySpec), GenerationsError> {
        let Some(previous_generation) = self.current_gen() else {
            unreachable!("current generation is only unavailable before any generation was added")
        };

        if next_generation == previous_generation {
            return Err(GenerationsError::RollbackToCurrentGeneration);
        }

        // we assume to track generations consecutively, 1 ..= total_generations
        if *next_generation > self.total_generations {
            return Err(GenerationsError::GenerationNotFound(*next_generation));
        };

        let history_spec = HistorySpec {
            author,
            hostname,
            command: Self::parse_argv(argv),
            timestamp,
            previous_generation: Some(previous_generation),
            current_generation: next_generation,
            kind: HistoryKind::SwitchGeneration,
        };

        // add action to history
        self.history.0.push(history_spec.into());

        let history_ref = self
            .history
            .0
            .iter()
            .next_back()
            .expect("history event should have been inserted");

        Ok((next_generation, history_ref))
    }

    /// Parse ARGV to store in a `HistorySpec`.
    ///
    /// If empty, as invokved from a unit test, return `None`.
    ///
    /// If non-empty, replace the first index with `flox` because we don't need
    /// to print the full path if `flox` was invoked with `/usr/local/bin/flox`.
    fn parse_argv(mut argv: Vec<String>) -> Option<Vec<String>> {
        if argv.is_empty() {
            return None;
        }

        argv[0] = "flox".to_string();
        Some(argv)
    }

    /// Access the history without granting access to the field
    /// for possible modification outside of this module.
    pub fn history(&self) -> &History {
        &self.history
    }

    /// Get the current live generation id
    pub fn current_gen(&self) -> Option<GenerationId> {
        self.history
            .iter()
            .next_back()
            .map(|spec| spec.current_generation)
    }

    /// Filter and reduce history to a table of generations,
    /// represented as [SingleGenerationMetadata].
    /// This function requires a semantically consistent history, i.e.
    ///
    /// * exactly one generation creating event for every generation [1..]
    /// * previous_generation correctly refers to the previously live generation
    ///
    /// These invariants are maintained when using [Self::add_generation]
    /// and [Self::switch_generation].
    pub fn generations(&self) -> BTreeMap<GenerationId, SingleGenerationMetadata> {
        let mut map: BTreeMap<GenerationId, SingleGenerationMetadata> = BTreeMap::new();
        for spec in self.history.iter() {
            match spec.kind {
                HistoryKind::SwitchGeneration => {
                    let prev = spec
                        .previous_generation
                        .and_then(|generation| map.get_mut(&generation))
                        .expect("there must be a previous generation by construction");
                    prev.last_live = Some(spec.timestamp);

                    let new = map
                        .get_mut(&spec.current_generation)
                        .expect("there must be a current generation by construction");
                    new.last_live = None;
                },
                _ => {
                    // Adding a generation performs an implicit switch.
                    // Hence, record that the previous generation was only live
                    // until the creation of the new generation.
                    if let Some(prev) = spec
                        .previous_generation
                        .and_then(|generation| map.get_mut(&generation))
                    {
                        prev.last_live = Some(spec.timestamp);
                    }

                    map.insert(spec.current_generation, SingleGenerationMetadata {
                        parent: spec.previous_generation,
                        created: spec.timestamp,
                        last_live: None,
                        description: spec.summary(),
                    });
                },
            }
        }

        map
    }
}

/// Metadata for a single generation of an environment
#[derive(Clone, Debug, PartialEq)]
pub struct SingleGenerationMetadata {
    pub parent: Option<GenerationId>,

    /// unix timestamp of the creation time of this generation
    pub created: DateTime<Utc>,

    /// unix timestamp of the time when this generation was last set as live
    /// `None` if this generation has never been set as live
    pub last_live: Option<DateTime<Utc>>,

    /// log message(s) describing the change from the previous generation
    pub description: String,
}

impl SingleGenerationMetadata {
    /// Create a new generation metadata instance
    pub fn new(description: String) -> Self {
        Self {
            parent: None,
            created: Utc::now(),
            last_live: None,
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
    Hash,
    Default,
    derive_more::Deref,
    derive_more::DerefMut,
    derive_more::From,
    derive_more::Display,
    DeserializeFromStr,
    SerializeDisplay,
    JsonSchema,
)]
#[schemars(try_from = "String")]
pub struct GenerationId(usize);

impl FromStr for GenerationId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(GenerationId(s.parse::<usize>().map_err(|_| {
            "generations must be referenced by number".to_string()
        })?))
    }
}

/// The type of history event that is associated with a change.
/// These are generation _creating_ changes (such as install, edit, etc.)
/// and metadata only changes such as switching generations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[serde(tag = "kind")]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
#[skip_serializing_none]
pub enum HistoryKind {
    #[schemars(title = "Import")]
    Import,
    #[schemars(title = "MigrateV1")]
    MigrateV1 { description: String },

    #[schemars(title = "Install")]
    Install { targets: Vec<String> },
    #[schemars(title = "Edit")]
    Edit,
    #[schemars(title = "Uninstall")]
    Uninstall { targets: Vec<String> },
    #[schemars(title = "Upgrade")]
    Upgrade { targets: Vec<String> },

    #[schemars(title = "IncludeUpgrade")]
    IncludeUpgrade { targets: Vec<String> },

    #[schemars(title = "SwitchGeneration")]
    SwitchGeneration,

    #[schemars(title = "Other")]
    Other { summary: String },

    #[serde(untagged)]
    #[schemars(title = "Unknown")]
    Unknown { kind: String },
}

#[derive(
    Debug, Clone, PartialEq, Serialize, derive_more::Deref, derive_more::DerefMut, JsonSchema,
)]
#[schemars(with = "T")]
pub struct WithOtherFields<T> {
    #[deref]
    #[deref_mut]
    #[serde(flatten)]
    inner: T,
    #[serde(flatten)]
    other: BTreeMap<String, Value>,
}

impl<T> From<T> for WithOtherFields<T> {
    fn from(value: T) -> Self {
        WithOtherFields {
            inner: value,
            other: Default::default(),
        }
    }
}

impl<'de, T> Deserialize<'de> for WithOtherFields<T>
where
    T: Deserialize<'de> + Serialize,
{
    fn deserialize<D>(deserializer: D) -> Result<WithOtherFields<T>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;

        #[derive(Deserialize)]
        struct WithOtherFieldsHelper<T> {
            #[serde(flatten)]
            inner: T,
            #[serde(flatten)]
            other: BTreeMap<String, Value>,
        }

        let mut helper = WithOtherFieldsHelper::deserialize(deserializer)?;
        // remove all fields present in the inner struct from the other fields, this is to avoid
        // duplicate fields in the catch all other fields because serde flatten does not exclude
        // already deserialized fields when deserializing the other fields.
        if let Value::Object(map) = serde_json::to_value(&helper.inner).map_err(D::Error::custom)? {
            for key in map.keys() {
                helper.other.remove(key);
            }
        }

        Ok(WithOtherFields {
            inner: helper.inner,
            other: helper.other,
        })
    }
}

/// The structure of a single change, tying together
/// _who_ performed _what_ change, where and when.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, JsonSchema)]
#[skip_serializing_none]
pub struct HistorySpec {
    // change provided
    /// Type of the change
    #[serde(flatten)]
    pub kind: HistoryKind,

    // system provided
    /// Local username of the user performing the change
    pub author: String,
    /// Hostname of the machine, on which the change was made
    pub hostname: String,
    /// Command line args to the command that performed the change
    /// This can be `None` if the change was invoked by a unit test or FloxHub.
    pub command: Option<Vec<String>>,
    /// Timestamp associated with the change
    // for consistency with the existing SingleGenerationMetadata
    #[serde(with = "chrono::serde::ts_seconds")]
    #[schemars(with = "usize")]
    pub timestamp: DateTime<Utc>,

    // associated generation(s)
    /// Currently live generation, e.g. created by the change
    /// or switched to.
    pub current_generation: GenerationId,
    /// Previous generation before a new generation was created,
    /// or the generation live before a generation switch.
    pub previous_generation: Option<GenerationId>,
}

impl HistorySpec {
    /// A short summary of the change.
    /// Summaries generally try to capture the _intent_ of the change
    /// rather than giving an exhaustive account about the complete change.
    /// The summary can be used alongside additional information such as author,
    /// host, diffs and diff derived information to produce richer change logs.
    pub fn summary(&self) -> String {
        fn format_targets(verb: &str, object: &str, targets: &[String]) -> String {
            let plural_s = if targets.len() < 2 { "" } else { "s" };

            let targets = targets
                .iter()
                .map(|target| format!("'{target}'"))
                .join(", ");

            format!("{verb} {object}{plural_s} {targets}")
        }

        fn format_targets_all_if_empty(verb: &str, object: &str, targets: &[String]) -> String {
            if targets.is_empty() {
                format!("{verb} all {object}s")
            } else {
                format_targets(verb, object, targets)
            }
        }

        match &self.kind {
            HistoryKind::Import => "imported environment".to_string(),
            HistoryKind::MigrateV1 { description } => {
                format!("{description} [metadata migrated]")
            },
            HistoryKind::Install { targets } => format_targets("installed", "package", targets),
            HistoryKind::Edit => "manually edited the manifest".to_string(),
            HistoryKind::Uninstall { targets } => format_targets("uninstalled", "package", targets),
            HistoryKind::Upgrade { targets } => {
                format_targets_all_if_empty("upgraded", "package", targets)
            },
            HistoryKind::IncludeUpgrade { targets } => {
                format_targets_all_if_empty("upgraded", "included environment", targets)
            },
            HistoryKind::SwitchGeneration => match self.previous_generation {
                Some(prev) => format!(
                    "changed current generation {prev} -> {}",
                    self.current_generation
                ),
                None => unreachable!(
                    "switch implementation prevents switches without a current live generation"
                ),
            },
            HistoryKind::Other { summary } => summary.to_string(),
            HistoryKind::Unknown { kind } => format!("performed unknown {kind} operation"),
        }
    }
}

#[derive(
    Debug, Clone, Serialize, Deserialize, Default, PartialEq, derive_more::AsRef, JsonSchema,
)]
pub struct History(Vec<WithOtherFields<HistorySpec>>);

impl<'h> IntoIterator for &'h History {
    type IntoIter = std::slice::Iter<'h, WithOtherFields<HistorySpec>>;
    type Item = &'h WithOtherFields<HistorySpec>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}
impl IntoIterator for History {
    type IntoIter = std::vec::IntoIter<WithOtherFields<HistorySpec>>;
    type Item = WithOtherFields<HistorySpec>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl History {
    pub fn iter(&self) -> <&Self as IntoIterator>::IntoIter {
        self.into_iter()
    }
}

mod compat {
    use std::collections::BTreeMap;

    use chrono::{DateTime, Utc};
    use flox_core::Version;
    use serde::{Deserialize, Serialize};
    use serde_with::{DeserializeFromStr, SerializeDisplay};

    use super::{AddGenerationOptions, GenerationsError, SwitchGenerationOptions};

    #[derive(
        Debug,
        Copy,
        Clone,
        PartialEq,
        Eq,
        PartialOrd,
        Ord,
        derive_more::Deref,
        derive_more::Display,
        derive_more::FromStr,
        derive_more::From,
        DeserializeFromStr,
        SerializeDisplay,
    )]
    pub struct GenerationId(usize);

    /// Metadata for a single generation of an environment
    #[derive(Deserialize, Serialize, Debug)]
    #[serde(rename_all = "camelCase")]
    pub struct SingleGenerationMetadata {
        /// unix timestamp of the creation time of this generation
        #[serde(with = "chrono::serde::ts_seconds")]
        pub created: DateTime<Utc>,

        /// unix timestamp of the time when this generation was last set as live
        /// `None` if this generation has never been set as live
        /// last_live is used in the new metadata format
        #[serde(with = "chrono::serde::ts_seconds_option")]
        pub last_active: Option<DateTime<Utc>>,

        /// log message(s) describing the change from the previous generation
        pub description: String,
        // todo: do we still need to track this?
        //       do we now?
        // /// store path of the built generation
        // path: PathBuf,
    }

    /// flox environment metadata for managed environments
    ///
    /// Managed environments support rolling back to previous generations.
    /// Generations are defined as immutable copy-on-write folders.
    /// Rollbacks and associated [SingleGenerationMetadata] are tracked per environment
    /// in a metadata file at the root of the environment branch.
    #[derive(Deserialize, Serialize, Debug, Default)]
    #[serde(rename_all = "camelCase")]
    pub struct AllGenerationsMetadataV1 {
        /// None means the environment has been created but does not yet have any
        /// generations
        pub current_gen: Option<GenerationId>,
        /// Metadata for all generations of the environment.
        /// Entries in this map must match up 1-to-1 with the generation folders
        /// in the environment branch.
        pub generations: BTreeMap<GenerationId, SingleGenerationMetadata>,
        /// Schema version of the metadata file, not yet utilized
        #[serde(default)]
        #[serde(rename = "version")]
        _version: Version<1>,
    }

    #[cfg(test)]
    impl AllGenerationsMetadataV1 {
        pub(super) fn new(
            current_gen: GenerationId,
            generations: BTreeMap<GenerationId, SingleGenerationMetadata>,
        ) -> Self {
            Self {
                current_gen: Some(current_gen),
                generations,
                _version: Version,
            }
        }
    }

    impl TryFrom<AllGenerationsMetadataV1> for super::AllGenerationsMetadata {
        type Error = GenerationsError;

        fn try_from(value: AllGenerationsMetadataV1) -> Result<Self, Self::Error> {
            let mut dest = super::AllGenerationsMetadata::default();

            for (index, (generation_id, generation_metadata)) in
                value.generations.iter().enumerate()
            {
                if (index + 1) != generation_id.0 {
                    Err(GenerationsError::MigrateV1ToV2(format!(
                        "metadata invalid gap in tracked generations detected, missing generation {}",
                        index + 1
                    )))?;
                }

                let add_generation_options = AddGenerationOptions {
                    author: "unknown".to_string(),
                    hostname: "unknown".to_string(),
                    argv: vec![],
                    timestamp: generation_metadata.created,
                    kind: super::HistoryKind::MigrateV1 {
                        description: generation_metadata.description.clone(),
                    },
                };

                dest.add_generation(add_generation_options);
            }

            let Some(original_current_gen) = value.current_gen else {
                Err(GenerationsError::MigrateV1ToV2(
                    "metadata has no current generation".into(),
                ))?
            };

            if Some(&*original_current_gen) != dest.current_gen().as_deref() {
                let timestamp = value
                    .generations
                    .get(&original_current_gen)
                    .ok_or(GenerationsError::MigrateV1ToV2(
                        "current generation missing".into(),
                    ))?
                    .last_active
                    .ok_or(GenerationsError::MigrateV1ToV2(
                        "current generation missing timestamp".into(),
                    ))?;

                let add_generation_options = SwitchGenerationOptions {
                    author: "unknown".to_string(),
                    hostname: "unknown".to_string(),
                    argv: vec![],
                    timestamp,
                    next_generation: super::GenerationId(*original_current_gen),
                };

                dest.switch_generation(add_generation_options)
                    .expect("switch to known generation should not fail");
            }

            Ok(dest)
        }
    }
}

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use std::sync::LazyLock;

    use chrono::Utc;

    use super::{AddGenerationOptions, GenerationId, HistoryKind, SwitchGenerationOptions};

    pub const AUTHOR: &str = "author";
    pub const HOSTNAME: &str = "host";
    pub static ARGV: LazyLock<Vec<String>> =
        LazyLock::new(|| vec!["flox".to_string(), "subcommand".to_string()]);

    pub fn default_add_generation_options() -> AddGenerationOptions {
        AddGenerationOptions {
            author: AUTHOR.into(),
            hostname: HOSTNAME.into(),
            argv: (*ARGV).clone(),
            timestamp: Utc::now(),
            kind: HistoryKind::Other {
                summary: "mock".into(),
            },
        }
    }

    pub fn default_switch_generation_options(
        next_generation: GenerationId,
    ) -> SwitchGenerationOptions {
        SwitchGenerationOptions {
            author: AUTHOR.into(),
            hostname: HOSTNAME.into(),
            argv: ARGV.clone(),
            timestamp: Utc::now(),
            next_generation,
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::flox::test_helpers::flox_instance;
    use crate::models::environment::Environment;
    use crate::models::environment::generations::test_helpers::{ARGV, AUTHOR, HOSTNAME};
    use crate::models::environment::path_environment::test_helpers::new_path_environment;

    const GEN_ID_1: GenerationId = GenerationId(1);
    const GEN_ID_2: GenerationId = GenerationId(2);

    mod metadata {
        use chrono::Utc;
        use pretty_assertions::{assert_eq, assert_str_eq};
        use serde_json::{Value, json};

        use crate::models::environment::generations::test_helpers::{
            ARGV,
            AUTHOR,
            HOSTNAME,
            default_add_generation_options,
            default_switch_generation_options,
        };
        use crate::models::environment::generations::{
            AllGenerationsMetadata,
            GenerationId,
            GenerationsError,
            HistoryKind,
            HistorySpec,
            SwitchGenerationOptions,
            WithOtherFields,
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

            let (generation, history) = metadata.add_generation(options.clone());
            let history = history.clone();

            let generations = metadata.generations();
            let generation_metadata = generations.get(&generation).expect("generation added");

            assert_eq!(metadata.current_gen(), Some(generation));
            assert_eq!(generation_metadata.created, options.timestamp);
            assert_eq!(generation_metadata.last_live, None);

            assert_eq!(history.author, options.author);
            assert_eq!(history.hostname, options.hostname);
            assert_eq!(history.current_generation, generation);
            assert_eq!(history.previous_generation, None);
            assert_eq!(history.kind, options.kind);
            assert_eq!(history.timestamp, options.timestamp);
        }

        #[test]
        fn generation_counter_is_correctly_increased() {
            let mut metadata = AllGenerationsMetadata::default();
            let (first_generation, _) = metadata.add_generation(default_add_generation_options());

            let (second_generation, _) = metadata.add_generation(default_add_generation_options());

            assert_eq!(first_generation, GenerationId(1));
            assert_eq!(second_generation, GenerationId(2));

            metadata
                .switch_generation(default_switch_generation_options(first_generation))
                .unwrap();
            assert_eq!(metadata.current_gen(), Some(first_generation));

            let (third_generation, _) = metadata.add_generation(default_add_generation_options());

            // generation counter continues at the current max (N=2) + 1
            assert_eq!(third_generation, GenerationId(3));
        }

        /// Switching generations
        ///
        /// * updates the current generation
        /// * updares the "last_live" timestamp of the switched to generation
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
                    metadata.current_gen(),
                    Some(generation_switched_to),
                    "current gen was not updated"
                );
                assert_eq!(
                    metadata.generations()[&generation_switched_to].last_live,
                    None,
                    "timestamp was not updated"
                );
                assert_eq!(
                    metadata.generations()[&generation_switched_from].last_live,
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

        #[test]
        fn switch_generation_requires_existing_generation() {
            let mut metadata = AllGenerationsMetadata::default();
            metadata.add_generation(default_add_generation_options());
            let absent_gen_id = GenerationId(2);
            let result =
                metadata.switch_generation(default_switch_generation_options(absent_gen_id));

            assert!(
                matches!(result, Err(GenerationsError::GenerationNotFound(2))),
                "unexpected result {:?}",
                result
            )
        }

        #[test]
        fn history_summaries() {
            let all_targets = [];
            let single_target = ["a".to_string()];
            let multiple_targets = ["a".to_string(), "b".to_string()];
            let change_message_pairs = [
                (HistoryKind::Edit, "manually edited the manifest"),
                (
                    HistoryKind::SwitchGeneration,
                    "changed current generation 1 -> 2",
                ),
                (
                    HistoryKind::IncludeUpgrade {
                        targets: all_targets.to_vec(),
                    },
                    "upgraded all included environments",
                ),
                (
                    HistoryKind::IncludeUpgrade {
                        targets: single_target.to_vec(),
                    },
                    "upgraded included environment 'a'",
                ),
                (
                    HistoryKind::IncludeUpgrade {
                        targets: multiple_targets.to_vec(),
                    },
                    "upgraded included environments 'a', 'b'",
                ),
                // does not use "all" format
                (
                    HistoryKind::Install {
                        targets: all_targets.to_vec(),
                    },
                    "installed package ",
                ),
                (
                    HistoryKind::Install {
                        targets: single_target.to_vec(),
                    },
                    "installed package 'a'",
                ),
                (
                    HistoryKind::Install {
                        targets: multiple_targets.to_vec(),
                    },
                    "installed packages 'a', 'b'",
                ),
                // does not use "all" format
                (
                    HistoryKind::Uninstall {
                        targets: all_targets.to_vec(),
                    },
                    "uninstalled package ",
                ),
                (
                    HistoryKind::Uninstall {
                        targets: single_target.to_vec(),
                    },
                    "uninstalled package 'a'",
                ),
                (
                    HistoryKind::Uninstall {
                        targets: multiple_targets.to_vec(),
                    },
                    "uninstalled packages 'a', 'b'",
                ),
                (
                    HistoryKind::Upgrade {
                        targets: all_targets.to_vec(),
                    },
                    "upgraded all packages",
                ),
                (
                    HistoryKind::Upgrade {
                        targets: single_target.to_vec(),
                    },
                    "upgraded package 'a'",
                ),
                (
                    HistoryKind::Upgrade {
                        targets: multiple_targets.to_vec(),
                    },
                    "upgraded packages 'a', 'b'",
                ),
            ];

            for (change_kind, message) in change_message_pairs {
                let spec = HistorySpec {
                    kind: change_kind,
                    author: AUTHOR.to_string(),
                    hostname: HOSTNAME.to_string(),
                    command: Some((*ARGV).clone()),
                    timestamp: Utc::now(),
                    current_generation: 2.into(),
                    previous_generation: Some(1.into()),
                };
                let summary = spec.summary();
                assert_str_eq!(summary, message)
            }
        }

        fn make_value(payload: &Value) -> Value {
            let mut value = json! {{
                "author": AUTHOR,
                "hostname": HOSTNAME,
                "command": *ARGV.clone(),
                "timestamp": Utc::now().timestamp(),
                "current_generation": "2",
                "previous_generation": "1",
            }};

            value
                .as_object_mut()
                .unwrap()
                .extend(payload.as_object().unwrap().clone());

            value
        }

        /// Assure that different history kinds can be serialized
        /// and deserialized losslessly.
        /// Specifically, unsupported events (e.g. created by future versions of flox)
        /// should not be redacted.
        #[test]
        fn parse_history() {
            let payloads = [
                json! {{"kind": "migrate_v1", "description": "v1 description"}},
                json! {{"kind": "import"}},
                json! {{"kind": "edit"}},
                json! {{"kind": "install", "targets": []}},
                json! {{"kind": "uninstall", "targets": []}},
                json! {{"kind": "upgrade", "targets": []}},
                json! {{"kind": "include_upgrade", "targets": []}},
                json! {{"kind": "uninstall", "targets": []}},
                json! {{"kind": "switch_generation"}},
                json! {{"kind": "uninstall", "targets": []}},
                json! {{"kind": "other", "summary": "foobar" }},
                // additional unknown fields
                json! {{"kind": "other", "summary": "foobar", "and": "more" }},
                // an unknown kind
                json! {{"kind": "unknown kind" }},
            ];

            for payload in payloads {
                let value = make_value(&payload);
                let deserialized_from_value: WithOtherFields<HistorySpec> =
                    match serde_json::from_value(value.clone()) {
                        Ok(v) => v,
                        Err(err) => {
                            panic!("should parse as history spec\nvalue:{value}\nerror:{err}")
                        },
                    };
                let serialized_to_value = match serde_json::to_value(&deserialized_from_value) {
                    Ok(v) => v,
                    Err(err) => panic!("should serialize to value\n{err}"),
                };

                assert_eq!(
                    serialized_to_value, value,
                    "serialization to value lost information"
                );

                let serialized_to_string =
                    match serde_json::to_string_pretty(&deserialized_from_value) {
                        Ok(v) => v,
                        Err(err) => panic!("should serialize to string\n{err}"),
                    };

                let deserialized_from_string: WithOtherFields<HistorySpec> =
                    match serde_json::from_str(&serialized_to_string) {
                        Ok(v) => v,
                        Err(err) => panic!(
                            "should deserialize from string\n{serialized_to_string}\nerror:{err}"
                        ),
                    };

                assert_eq!(
                    deserialized_from_string, deserialized_from_value,
                    "serialization to string lost information"
                );
            }
        }

        #[test]
        fn unknown_kinds_and_data_are_allowed() {
            let payloads = [
                // unknown kind
                (
                    json! {{"kind": "fromthefuture", "not-targets": {}}},
                    HistoryKind::Unknown {
                        kind: "fromthefuture".to_string(),
                    },
                    &["not-targets"],
                ),
                // extra fields
                (
                    json! {{"kind": "install", "targets": [], "and-not-targets":[]}},
                    HistoryKind::Install {
                        targets: Default::default(),
                    },
                    &["and-not-targets"],
                ),
                //
                // wrong fields, doesn't parse as install becauise the fields don't match,
                // but won't fail parsing
                (
                    json! {{ "kind": "install", "targets": "not a list" }},
                    HistoryKind::Unknown {
                        kind: "install".to_string(),
                    },
                    &["targets"],
                ),
            ];

            for (payload, history_kind, other_fields) in payloads {
                let value = make_value(&payload);
                let with_other_fields =
                    serde_json::from_value::<WithOtherFields<HistorySpec>>(value.clone())
                        .unwrap_or_else(|_| panic!("{value} should parse"));

                assert_eq!(with_other_fields.kind, history_kind);

                let actual_extra_fields: Vec<&String> = with_other_fields.other.keys().collect();
                for expected_field in other_fields {
                    assert!(
                        actual_extra_fields
                            .iter()
                            .any(|field| field == expected_field),
                        "extra field {expected_field} not found"
                    );
                }
            }
        }
        #[test]
        fn invalid_data_is_not_allowed() {
            let payloads = [
                // wrong kind type
                json! {{ "kind": { "an object": "not a string" } }},
            ];

            for payload in payloads {
                let value = make_value(&payload);
                let _ = serde_json::from_value::<WithOtherFields<HistorySpec>>(value.clone())
                    .expect_err(&format!("{value} should fail to parse"));
            }
        }
    }

    mod compat {
        use std::collections::BTreeMap;

        use chrono::{DateTime, Duration, Utc};
        use indoc::indoc;
        use pretty_assertions::assert_eq;
        use serde_json::json;

        use crate::models::environment::generations::compat::{self, SingleGenerationMetadata};
        use crate::models::environment::generations::{
            AddGenerationOptions,
            AllGenerationsMetadata,
            HistoryKind,
            SwitchGenerationOptions,
            parse_metadata,
        };

        fn migration_options(timestamp: DateTime<Utc>) -> AddGenerationOptions {
            AddGenerationOptions {
                author: "unknown".to_string(),
                hostname: "unknown".to_string(),
                argv: vec![],
                timestamp,
                kind: HistoryKind::MigrateV1 {
                    description: "description".to_string(),
                },
            }
        }

        #[test]
        fn parse_v1() {
            let date = DateTime::default();

            let metadata = compat::AllGenerationsMetadataV1::new(
                1.into(),
                BTreeMap::from_iter([(1.into(), SingleGenerationMetadata {
                    created: date,
                    last_active: Some(date),
                    description: "description".to_string(),
                })]),
            );
            let serialized = serde_json::to_string_pretty(&metadata).unwrap();
            let actual =
                parse_metadata(&mut serde_json::Deserializer::from_str(&serialized)).unwrap();

            let mut expected = AllGenerationsMetadata::default();
            expected.add_generation(migration_options(date));

            assert_eq!(*actual, expected)
        }

        #[test]
        fn parse_v2_invalid() {
            let metadata = json!({
                "version": 2,
                // Missing required fields.
            });
            let serialized = serde_json::to_string_pretty(&metadata).unwrap();
            let err = parse_metadata(&mut serde_json::Deserializer::from_str(&serialized))
                .expect_err("invalid v2 schema should fail to parse");
            assert_eq!(
                err.to_string(),
                "Environment metadata of version '2' could not be parsed into its expected schema."
            );
        }

        #[test]
        fn parse_v3_unknown() {
            let metadata = json!({
                "version": 3,
            });
            let serialized = serde_json::to_string_pretty(&metadata).unwrap();
            let err = parse_metadata(&mut serde_json::Deserializer::from_str(&serialized))
                .expect_err("unknown v3 schema should fail to parse");
            assert_eq!(err.to_string(), indoc! {"
                Environment metadata of version '3' is not supported

                This environment appears to have been modified by a newer version of Flox.
                Please upgrade to the latest version of Flox and try again."});
        }

        #[test]
        fn metadata_migrations() {
            let date = DateTime::default();
            let pairs = [
                // a single generation
                (
                    compat::AllGenerationsMetadataV1::new(
                        1.into(),
                        BTreeMap::from_iter([(1.into(), SingleGenerationMetadata {
                            created: date,
                            last_active: Some(date),
                            description: "description".to_string(),
                        })]),
                    ),
                    {
                        let mut migrated = AllGenerationsMetadata::default();
                        migrated.add_generation(migration_options(date));
                        migrated
                    },
                ),
                // multiple generations linear
                (
                    compat::AllGenerationsMetadataV1::new(
                        3.into(),
                        BTreeMap::from_iter([
                            (1.into(), SingleGenerationMetadata {
                                created: date,
                                last_active: Some(date),
                                description: "description".to_string(),
                            }),
                            (2.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(1),
                                last_active: Some(date + Duration::hours(1)), // [sic]
                                description: "description".to_string(),
                            }),
                            (3.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(2),
                                last_active: Some(date + Duration::hours(2)), // [sic]
                                description: "description".to_string(),
                            }),
                        ]),
                    ),
                    {
                        let mut migrated = AllGenerationsMetadata::default();
                        migrated.add_generation(migration_options(date));
                        migrated.add_generation(migration_options(date + Duration::hours(1)));
                        migrated.add_generation(migration_options(date + Duration::hours(2)));
                        migrated
                    },
                ),
                // multiple generations with rollback
                // 1->2->1->3
                //
                // However we don't know when that rollback happened (and from where)
                //
                (
                    compat::AllGenerationsMetadataV1::new(
                        3.into(),
                        BTreeMap::from_iter([
                            (1.into(), SingleGenerationMetadata {
                                created: date,
                                last_active: Some(date + Duration::hours(2)),
                                description: "description".to_string(),
                            }),
                            (2.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(1),
                                last_active: Some(date + Duration::hours(1)), // [sic]
                                description: "description".to_string(),
                            }),
                            (3.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(3),
                                last_active: Some(date + Duration::hours(3)), // [sic]
                                description: "description".to_string(),
                            }),
                        ]),
                    ),
                    {
                        let mut migrated = AllGenerationsMetadata::default();

                        migrated.add_generation(migration_options(date));
                        migrated.add_generation(migration_options(date + Duration::hours(1)));
                        migrated.add_generation(migration_options(date + Duration::hours(3)));

                        migrated
                    },
                ),
                // multiple generations with rollback
                // 1->2->3->2
                //
                // we only know, 2 is the current generation,
                // so only final rollback is created to switch to the final generation
                (
                    compat::AllGenerationsMetadataV1::new(
                        2.into(),
                        BTreeMap::from_iter([
                            (1.into(), SingleGenerationMetadata {
                                created: date,
                                last_active: Some(date + Duration::hours(2)),
                                description: "description".to_string(),
                            }),
                            (2.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(1),
                                last_active: Some(date + Duration::hours(3)), // [sic]
                                description: "description".to_string(),
                            }),
                            (3.into(), SingleGenerationMetadata {
                                created: date + Duration::hours(2),
                                last_active: Some(date + Duration::hours(2)), // [sic]
                                description: "description".to_string(),
                            }),
                        ]),
                    ),
                    {
                        let mut migrated = AllGenerationsMetadata::default();

                        migrated.add_generation(migration_options(date));
                        let (second_generation, ..) =
                            migrated.add_generation(migration_options(date + Duration::hours(1)));
                        migrated.add_generation(migration_options(date + Duration::hours(2)));
                        migrated
                            .switch_generation(SwitchGenerationOptions {
                                author: "unknown".into(),
                                hostname: "unknown".into(),
                                argv: vec![],
                                timestamp: date + Duration::hours(3),
                                next_generation: second_generation,
                            })
                            .unwrap();
                        migrated
                    },
                ),
            ];

            for (v1, expected_v2) in pairs {
                let expected_current_gen = v1.current_gen;
                let actual_v2 = AllGenerationsMetadata::try_from(v1).unwrap();

                assert_eq!(actual_v2, expected_v2);
                assert_eq!(
                    *actual_v2.current_gen().unwrap(),
                    *expected_current_gen.unwrap()
                )
            }
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

        let mut generations_rw = generations
            .writable(&tempdir, AUTHOR, HOSTNAME, &ARGV)
            .unwrap();
        generations_rw
            .add_generation(&mut core_env, HistoryKind::Other {
                summary: "First generation".to_string(),
            })
            .unwrap();
        generations_rw
            .add_generation(&mut core_env, HistoryKind::Other {
                summary: "Second generation".to_string(),
            })
            .unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen(),
            Some(GEN_ID_2),
            "should be at second generation"
        );

        (generations, tempdir)
    }

    #[test]
    fn set_current_generation_backwards_and_forwards() {
        let (mut generations, tempdir) = setup_two_generations();
        let mut generations_rw = generations
            .writable(&tempdir, AUTHOR, HOSTNAME, &ARGV)
            .unwrap();

        assert_eq!(
            generations_rw.metadata().unwrap().current_gen(),
            Some(GEN_ID_2),
            "should start at second generation"
        );

        generations_rw.set_current_generation(GEN_ID_1).unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen(),
            Some(GEN_ID_1),
            "should roll back to first generation"
        );

        generations_rw.set_current_generation(GEN_ID_2).unwrap();
        assert_eq!(
            generations_rw.metadata().unwrap().current_gen(),
            Some(GEN_ID_2),
            "should roll forwards to second generation"
        );
    }

    #[test]
    fn set_current_generation_not_found() {
        let (mut generations, tempdir) = setup_two_generations();
        let mut generations_rw = generations
            .writable(&tempdir, AUTHOR, HOSTNAME, &ARGV)
            .unwrap();

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
        let mut generations_rw = generations
            .writable(&tempdir, AUTHOR, HOSTNAME, &ARGV)
            .unwrap();

        let current_gen = generations_rw.metadata().unwrap().current_gen().unwrap();
        let res = generations_rw.set_current_generation(current_gen);
        assert!(
            matches!(res, Err(GenerationsError::RollbackToCurrentGeneration)),
            "should error when setting current generation, got: {:?}",
            res
        );
    }
}

#[test]
#[ignore = "only exporting schema"]
fn export_schema() {
    use std::fs::File;
    use std::io::Write;
    let schema = schemars::schema_for!(AllGenerationsMetadata);

    // Slightly hacky since we cant read the target dir
    // or even at least the workspace dir directly:
    // <https://github.com/rust-lang/cargo/issues/3946>
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let schemars_basedir = manifest_dir.join("../target/schemars");
    fs::create_dir_all(&schemars_basedir).unwrap();

    let schema_path = schemars_basedir.join("generations-metadata-v2.schema.json");
    let mut schema_file = File::create(&schema_path).unwrap();

    writeln!(&mut schema_file, "{:#}", schema.as_value()).unwrap();

    println!("schema written to {schema_path:?}")
}
