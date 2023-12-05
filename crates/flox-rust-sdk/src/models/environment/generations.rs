#![allow(dead_code)] //
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use flox_types::version::Version;
use serde::{Deserialize, Serialize};
use serde_with::{DeserializeFromStr, SerializeDisplay};
use thiserror::Error;

use super::path_environment::{Original, PathEnvironment};
use super::{copy_dir_recursive, PathPointer, ENV_DIR_NAME, LOCKFILE_FILENAME, MANIFEST_FILENAME};
use crate::providers::git::{GitCommandProvider, GitProvider};

const GENERATIONS_METADATA_FILE: &str = "metadata.json";

/// A representation of the generations of an environment
///
/// Example File layout:
///
/// ./
/// ├── 1
/// │  └── env
/// │     ├── manifest.toml
/// │     └── manifest.lock
/// ├── 2
/// │  └── env
/// │    └── manifest.toml (lockfile is optional)
/// ├── ... N
/// │  └── env
/// │     └── manifest.toml
/// └── metadata.json
pub struct Generations {
    /// A floxmeta repository/branch that contains the generations of an environment
    repo: GitCommandProvider,
    branch: String,

    /// A path pointer for the environment that will
    /// be associated with a realized generation
    pointer: PathPointer,

    /// A temporary directory that will be used to realize generations into
    tempdir_base: PathBuf,
}

impl Generations {
    /// Create a new generations instance
    pub fn new(
        environment_repr: GitCommandProvider,
        ref_name: String,
        pointer: PathPointer,
        tempdir_base: PathBuf,
    ) -> Self {
        Self {
            repo: environment_repr,
            branch: ref_name,
            pointer,
            tempdir_base,
        }
    }

    /// Read the generations metadata for an environment
    pub fn metadata(&self) -> Result<Metadata, GenerationsError> {
        read_metadata(&self.repo, &self.branch)
    }

    /// Realize the generations branch into a temporary directory
    fn realize(&self) -> Result<GitCommandProvider, GenerationsError> {
        let git_options = self.repo.get_options().clone();

        let realized_path = tempfile::tempdir_in(&self.tempdir_base)
            .unwrap()
            .into_path();

        let repo = GitCommandProvider::clone_branch_with(
            git_options,
            self.repo.path(),
            realized_path,
            &self.branch,
            false,
        )
        .unwrap();

        Ok(repo)
    }

    /// Realize a generation as a [PathEnvironment]
    ///
    /// The generation can then be safely modified
    /// and registered as a new generation using [Self::add_generation].
    pub fn realize_generation(
        &self,
        generation: usize,
    ) -> Result<PathEnvironment<Original>, GenerationsError> {
        let realized_gens = self.realize()?;

        let tempdir = tempfile::tempdir_in(&self.tempdir_base)
            .unwrap()
            .into_path();
        let environment = PathEnvironment::new(
            realized_gens.path().join(generation.to_string()),
            self.pointer.clone(),
            tempdir,
            Original,
        )
        .unwrap();

        Ok(environment)
    }

    /// Realize the current generation as set in the metadata file
    /// as a [PathEnvironment].
    ///
    /// The generation can then be safely modified
    /// and registered as a new generation using [Self::add_generation].
    pub fn realize_current_generation(
        &self,
    ) -> Result<PathEnvironment<Original>, GenerationsError> {
        let metadata = self.metadata()?;
        let current_gen = metadata
            .current_gen
            .ok_or(GenerationsError::NoGenerations)?;
        self.realize_generation(*current_gen)
    }

    /// Import an existing environment into a generation
    ///
    /// Assumes the invariant that the [PathEnvironment] instance is valid.
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
        environment: PathEnvironment<Original>,
        generation: usize,
        generation_metadata: GenerationMetadata,
        set_current: bool,
    ) -> Result<(), GenerationsError> {
        let realized = self.realize()?;

        let description = generation_metadata.description.clone();

        let mut metadata = self.metadata()?;
        let _existing = metadata
            .generations
            .insert(generation.into(), generation_metadata);

        if set_current {
            metadata.current_gen = Some(generation.into());
        }

        write_metadata_file(metadata, realized.path())?;

        let generation_path = realized.path().join(generation.to_string());
        let env_path = generation_path.join(ENV_DIR_NAME);
        fs::create_dir_all(&env_path).unwrap();

        // copy `env/`, i.e. manifest and lockfile (if it exists) and possibly other assets
        // copy into `<generation>/env/` to make creating `PathEnvironment` easier
        copy_dir_recursive(&environment.path.join(ENV_DIR_NAME), &env_path, true).unwrap();

        realized.add(&[Path::new(".")]).unwrap();
        realized
            .commit(&format!(
                "Register generation {}\n\n{}",
                generation, description
            ))
            .unwrap();
        realized.push("origin", false).unwrap();

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
        environment: PathEnvironment<Original>,
        generation_metadata: GenerationMetadata,
    ) -> Result<(), GenerationsError> {
        // keys should all be numbers (but)
        let max = self
            .metadata()?
            .generations
            .keys()
            .cloned()
            .max()
            .unwrap_or_default();

        self.register_generation(environment, *max + 1, generation_metadata, true)
    }

    /// Switch to a provided generation.
    ///
    /// Fails if the generation does not exist.
    ///
    /// This method will not perform any validation of the generation switched to.
    /// If validation (e.g. proving that the environment builds) is required,
    /// it should first be realized using [Self::realize_generation].
    fn set_current_generation(&mut self, generation: usize) -> Result<(), GenerationsError> {
        let mut metadata = self.metadata()?;
        let realized = self.realize()?;

        let generation_metadata = metadata.generations.contains_key(&generation.into());
        if !generation_metadata {
            return Err(GenerationsError::GenerationNotFound(generation));
        }

        metadata.current_gen = Some(generation.into());

        write_metadata_file(metadata, realized.path())?;

        realized.add(&[Path::new(".")]).unwrap();
        realized
            .commit(&format!("Set current generation to {}", generation))
            .unwrap();
        realized.push("origin", false).unwrap();

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

/// Reads the generations metadata file directly from the repository
fn read_metadata(repo: &GitCommandProvider, ref_name: &str) -> Result<Metadata, GenerationsError> {
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
fn write_metadata_file(metadata: Metadata, realized_path: &Path) -> Result<(), GenerationsError> {
    let metadata_content = serde_json::to_string(&metadata).unwrap();
    let metadata_path = realized_path.join(GENERATIONS_METADATA_FILE);
    fs::write(metadata_path, metadata_content).unwrap();
    Ok(())
}

/// flox environment metadata for managed environments
///
/// Managed environments support rolling back to previous generations.
/// Generations are defined as immutable copy-on-write folders.
/// Rollbacks and associated [GenerationMetadata] are tracked per environment
/// in a metadata file at the root of the environment branch.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    /// None means the environment has been created but does not yet have any
    /// generations
    current_gen: Option<GenerationId>,
    /// Metadata for all generations of the environment.
    /// Entries in this map must match up 1-to-1 with the generation folders
    /// in the environment branch.
    generations: BTreeMap<GenerationId, GenerationMetadata>,
    /// Schema version of the metadata file, not yet utilized
    #[serde(default)]
    version: Version<1>,
}

/// Metadata for a single generation of an environment
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GenerationMetadata {
    /// unix timestamp of the creation time if this generation
    #[serde(with = "chrono::serde::ts_seconds")]
    created: DateTime<Utc>,

    /// log message(s) describing the change from the previous generation
    description: String,
    // todo: do we still need to track this?
    //       do we now?
    // /// store path of the built generation
    // path: PathBuf,
}

impl GenerationMetadata {
    /// Create a new generation metadata instance
    pub fn new(description: String) -> Self {
        Self {
            created: Utc::now(),
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
