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
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flox_types::version::Version;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use thiserror::Error;

use super::core_environment::CoreEnvironment;
use super::{copy_dir_recursive, ENV_DIR_NAME};
use crate::models::environment::MANIFEST_FILENAME;
use crate::providers::git::{GitCommandProvider, GitProvider};

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
/// Mutating commands are commited and pushed to the [ReadOnly] branch.
///
/// Instances of this type are created using [Generations::writable].
///
/// Todo: rename to `CheckedOut`, `Filesystem`, ...?
///
/// /// See also: [ReadOnly]
pub struct ReadWrite {}

/// A representation of the generations of an environment
///
/// Essentially the branches in a Floxmeta repository.
///
/// Todo: merge with or integrate Floxmeta?
pub struct Generations<State = ReadOnly> {
    /// A floxmeta repository/branch that contains the generations of an environment
    ///
    /// - When [ReadOnly], this is assumend to be bare, but it is not enforced.
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
            .unwrap();

        return Ok(manifest_osstr.to_string_lossy().to_string());
    }

    /// Read the manifest of the current generation and return its contents as a string
    pub fn current_gen_manifest(&self) -> Result<String, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;

        self.manifest(*current_gen)
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

    pub fn writable(
        self,
        tempdir: impl AsRef<Path>,
    ) -> Result<Generations<ReadWrite>, GenerationsError> {
        let repo = checkout_to_tempdir(
            &self.repo,
            &self.branch,
            tempfile::tempdir_in(tempdir).unwrap().into_path(),
        )?;

        Ok(Generations {
            repo,
            branch: self.branch,
            _state: ReadWrite {},
        })
    }
}

impl Generations<ReadWrite> {
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
    pub fn get_generation(&self, generation: usize) -> Result<CoreEnvironment, GenerationsError> {
        let environment = CoreEnvironment::new(
            self.repo
                .path()
                .join(generation.to_string())
                .join(ENV_DIR_NAME),
        );

        Ok(environment)
    }

    /// Return the current generation as set in the metadata file
    /// as a [CoreEnvironment].
    ///
    /// The generation can then be safely modified
    /// and registered as a new generation using [Self::add_generation].
    pub fn get_current_generation(&self) -> Result<CoreEnvironment, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;
        self.get_generation(*current_gen)
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
        environment: CoreEnvironment,
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

        let _existing = metadata
            .generations
            .insert(generation.into(), generation_metadata);

        write_metadata_file(metadata, self.repo.path())?;

        let generation_path = self.repo.path().join(generation.to_string());
        let env_path = generation_path.join(ENV_DIR_NAME);
        fs::create_dir_all(&env_path).unwrap();

        // copy `env/`, i.e. manifest and lockfile (if it exists) and possibly other assets
        // copy into `<generation>/env/` to make creating `PathEnvironment` easier
        copy_dir_recursive(&environment.path(), &env_path, true).unwrap();

        self.repo.add(&[&generation_path]).unwrap();
        self.repo
            .commit(&format!(
                "Create generation {}\n\n{}",
                generation, description
            ))
            .unwrap();
        self.repo.push("origin", false).unwrap();

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
        environment: CoreEnvironment,
        description: String,
    ) -> Result<(), GenerationsError> {
        // keys should all be numbers (but)
        let max = self
            .metadata()?
            .generations
            .keys()
            .cloned()
            .max()
            .unwrap_or_default();

        self.register_generation(environment, *max + 1, description, true)
    }

    /// Switch to a provided generation.
    ///
    /// Fails if the generation does not exist.
    ///
    /// This method will not perform any validation of the generation switched to.
    /// If validation (e.g. proving that the environment builds) is required,
    /// it should first be realized using [Self::realize_generation].
    #[allow(unused)]
    fn set_current_generation(&mut self, generation: usize) -> Result<(), GenerationsError> {
        let mut metadata = self.metadata()?;

        let generation_metadata = metadata.generations.contains_key(&generation.into());
        if !generation_metadata {
            return Err(GenerationsError::GenerationNotFound(generation));
        }

        metadata.current_gen = Some(generation.into());

        write_metadata_file(metadata, self.repo.path())?;

        self.repo
            .add(&[Path::new(GENERATIONS_METADATA_FILE)])
            .unwrap();
        self.repo
            .commit(&format!("Set current generation to {}", generation))
            .unwrap();
        self.repo.push("origin", false).unwrap();

        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum GenerationsError {
    #[error("Generation {0} not found")]
    GenerationNotFound(usize),

    #[error("No generations found in environment")]
    NoGenerations,
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
            .unwrap();

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
            .unwrap();
        serde_json::from_str(&metadata_content.to_string_lossy()).unwrap()
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
    let metadata_content = serde_json::to_string(&metadata).unwrap();
    let metadata_path = realized_path.join(GENERATIONS_METADATA_FILE);
    fs::write(metadata_path, metadata_content).unwrap();
    Ok(())
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
    // todo: tests for this will be easier with the `init` method implemented
    // in https://github.com/flox/flox/pull/563
}
