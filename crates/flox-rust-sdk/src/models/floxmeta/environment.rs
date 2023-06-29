use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{TimeZone, Utc};
use itertools::Itertools;
use log::debug;
use runix::flake_ref::git::{GitAttributes, GitRef};
use runix::flake_ref::FlakeRef;
use runix::installable::Installable;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

use super::{Floxmeta, GetFloxmetaError, TransactionCommitError, TransactionEnterError};
use crate::models::environment::{DEFAULT_KEEP_GENERATIONS, DEFAULT_MAX_AGE_DAYS};
use crate::models::environment_ref::Named;
use crate::models::root::transaction::{GitAccess, GitSandBox, ReadOnly};
use crate::providers::git::{BranchInfo, GitProvider};

pub const METADATA_JSON: &'_ str = "metadata.json";

#[derive(Serialize, Debug)]
pub struct Environment<'flox, Git: GitProvider, A: GitAccess<Git>> {
    name: String,
    system: String,
    remote: Option<EnvBranch>,
    local: Option<EnvBranch>,
    #[serde(skip)]
    floxmeta: Floxmeta<'flox, Git, A>,
}

#[derive(Serialize, Debug, Clone)]
#[allow(unused)]
pub struct EnvBranch {
    description: String,
    hash: String,
}

/// flox environment metadata for managed environments
///
/// Managed environments support rolling back to previous generations.
/// Generations are defined immutable copy-on-write folders.
/// Rollbacks and asssociated [GenerationMetadata] is tracked per environemnt
/// in a metadata file at the root of the environment branch.
#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    /// None means the environment has been created but does not yet have any
    /// generations
    pub current_gen: Option<String>,
    /// Metadata for all generations of the environment.
    /// Entries in this map must match up 1-to-1 with the generation folders
    /// in the environment branch.
    generations: BTreeMap<String, GenerationMetadata>,
    /// Schema version of the metadata file, not yet utilized
    #[serde(default)]
    version: u32,
}

/// Metadata for a single generation of an environment
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GenerationMetadata {
    /// unix timestamp of the creation time if this generation
    created: u64,
    /// unix timestamp of the last activation.
    /// taken into account during garbage collection
    last_active: u64,
    /// log message(s) describing the change from the previous generation
    log_message: Vec<String>,
    /// store path of the built generation
    path: PathBuf,
    /// Schema version of the metadata file, not yet utilized
    /// TODO: equivalent to metadata.json#version?
    #[serde(default)]
    version: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Manifest {
    version: i64,
    elements: Vec<Element>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    active: bool,
    store_paths: Vec<String>,
    priority: Option<i64>,
    #[serde(flatten)]
    source: Option<ElementSource>,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ElementSource {
    attr_path: String,
    outputs: Option<Vec<String>>,
    url: String,
    original_url: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Generation {
    name: String,
    metadata: GenerationMetadata,
    elements: Vec<Element>,
}

/// Implementations for an opened floxmeta
impl<'flox, Git: GitProvider, A: GitAccess<Git>> Floxmeta<'flox, Git, A> {
    pub async fn environment(
        &self,
        name: &str,
    ) -> Result<Environment<'flox, Git, ReadOnly<Git>>, GetEnvironmentError<Git>> {
        self.environments()
            .await?
            .into_iter()
            .find(|env| env.name == name && env.system == self.flox.system)
            .ok_or(GetEnvironmentError::NotFound)
    }

    /// Detect all environments from a floxmeta repo
    pub async fn environments(
        &self,
    ) -> Result<Vec<Environment<'flox, Git, ReadOnly<Git>>>, GetEnvironmentsError<Git>> {
        self.access
            .git()
            .fetch()
            .await
            .map_err(GetEnvironmentsError::FetchBranches)?;

        // get output of `git branch -av`
        let list_branches_output = self
            .access
            .git()
            .list_branches()
            .await
            .map_err(GetEnvironmentsError::ListBranches)?;

        // parse and sort git output
        let environments = list_branches_output
            .into_iter()
            .filter_map(|branch| {
                // discard unknown remote
                match branch.remote.as_deref() {
                    Some("origin") | None => Some(branch),
                    Some(remote) => {
                        debug!(
                            "Unknown remote '{remote}' for branch '{name}', \
                             not listing environment",
                            remote = remote,
                            name = branch.name
                        );
                        None
                    },
                }
            })
            .filter_map(|branch| {
                // discard unknown branch names
                let (system, name) = branch.name.split_once('.').or_else(|| {
                    debug!(
                        "Branch '{name}' does not look like \
                            an environment branch ('<system>.<name>'), \
                            not listing environment",
                        name = branch.name
                    );
                    None
                })?;

                let branch = BranchInfo {
                    name: name.to_string(),
                    remote: branch.remote,
                    rev: branch.rev,
                    description: branch.description,
                };
                Some((system.to_string(), branch))
            })
            .map(|(system, branch)| {
                // wrap into an environment branch struct
                let env_branch = EnvBranch {
                    description: branch.description,
                    hash: branch.rev,
                };

                let (local, remote) = if branch.remote.is_some() {
                    (None, Some(env_branch))
                } else {
                    (Some(env_branch), None)
                };

                Environment {
                    name: branch.name,
                    system,
                    local,
                    remote,
                    floxmeta: Floxmeta {
                        owner: self.owner.clone(),
                        flox: self.flox,
                        access: self.access.read_only(),
                        _git: std::marker::PhantomData,
                    },
                }
            })
            .fold(
                HashMap::new(),
                |mut merged: HashMap<_, Environment<_, _>>, mut env| {
                    // emplace either remote or local in a reference we already stored
                    // assumes only at most one of each is present
                    if let Some(stored) = merged.get_mut(&(env.name.clone(), env.system.clone())) {
                        stored.local = env.local.take().or_else(|| stored.local.take());
                        stored.remote = env.remote.take().or_else(|| stored.remote.take());
                    }
                    // if its the first reference for the <system, <name> combination
                    // store it to be possibly merged
                    else {
                        merged.insert((env.name.clone(), env.system.clone()), env);
                    }
                    merged
                },
            )
            .into_values()
            .collect();
        Ok(environments)
    }
}

/// Implementations for an environment
impl<'flox, Git: GitProvider> Environment<'flox, Git, ReadOnly<Git>> {
    /// The name of the environment
    pub fn name(&self) -> Cow<str> {
        Cow::from(&self.name)
    }

    /// The platform this environment can be activated on
    pub fn system(&self) -> Cow<str> {
        Cow::from(&self.system)
    }

    pub fn owner(&self) -> Cow<str> {
        Cow::from(&self.floxmeta.owner)
    }

    pub fn remote(&self) -> Cow<Option<EnvBranch>> {
        Cow::Borrowed(&self.remote)
    }

    pub fn local(&self) -> Cow<Option<EnvBranch>> {
        Cow::Borrowed(&self.local)
    }

    pub fn as_env_ref(&self) -> Named {
        Named {
            name: self.name.to_string(),
            owner: self.floxmeta.owner.to_string(),
        }
    }

    fn symlink_path(&self, generation: &str) -> PathBuf {
        let owner_dir = Named::associated_owner_dir(self.floxmeta.flox, &self.floxmeta.owner);
        owner_dir.join(format!(
            "{system}.{name}-{generation}-link",
            system = self.system,
            name = self.name
        ))
    }

    pub async fn metadata(&self) -> Result<Metadata, MetadataError<Git>> {
        let git = &self.floxmeta.access.git();
        let metadata_str = git
            .show(&format!("{}.{}:{}", self.system, self.name, METADATA_JSON))
            .await
            .map_err(MetadataError::RetrieveMetadata)?;

        let metadata: Metadata = serde_json::from_str(&metadata_str.to_string_lossy())
            .map_err(MetadataError::ParseMetadata)?;

        Ok(metadata)
    }

    pub async fn generation(
        &self,
        generation: Option<&str>,
    ) -> Result<Generation, GenerationError<Git>> {
        let git = &self.floxmeta.access.git();
        let mut metadata = self.metadata().await?;

        let generation = generation
            .or(metadata.current_gen.as_deref())
            .ok_or(GenerationError::Empty)?;

        let generation_metadata = metadata
            .generations
            .remove(generation)
            .ok_or(GenerationError::NotFound)?;
        let manifest_content = git
            .show(&format!(
                "{}.{}:{}/{}",
                self.system, self.name, generation, "manifest.json"
            ))
            .await
            .map_err(ManifestError::RetrieveManifest)?;

        let manifest: Manifest = serde_json::from_str(&manifest_content.to_string_lossy())
            .map_err(ManifestError::ParseManifest)?;

        Ok(Generation {
            name: generation.to_owned(),
            metadata: generation_metadata,
            elements: manifest.elements,
        })
    }

    pub async fn installable(
        &self,
        generation: Option<&str>,
    ) -> Result<Installable, GenerationError<Git>> {
        let git = &self.floxmeta.access.git();
        let metadata = self.metadata().await?;

        let generation = generation
            .or(metadata.current_gen.as_deref())
            .ok_or(GenerationError::Empty)?;

        let flakeref = FlakeRef::GitPath(GitRef {
            // we can unwrap here since we construct and know the path
            url: Url::from_file_path(git.path()).unwrap().try_into().unwrap(),
            attributes: GitAttributes {
                reference: format!("{system}.{name}", system = self.system(), name = self.name)
                    .into(),
                dir: Path::new(generation).to_path_buf().into(),
                ..Default::default()
            },
        });

        Ok(Installable {
            flakeref,
            attr_path: ["", "floxEnvs", "default"].try_into().unwrap(),
        })
    }

    pub async fn delete_symlinks(&self) -> Result<bool, DeleteSymlinksError<Git>> {
        let mut symlinks_to_delete = self.symlinks_to_delete(self.metadata().await?).peekable();
        if symlinks_to_delete.peek().is_some() {
            for symlink in self.symlinks_to_delete(self.metadata().await?) {
                fs::remove_file(symlink?).unwrap();
            }
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Returns iterator of symlinks for old generations, keeping at least
    /// DEFAULT_KEEP_GENERATIONS
    fn symlinks_to_delete(
        &self,
        metadata: Metadata,
    ) -> impl Iterator<Item = Result<PathBuf, DeleteSymlinksError<Git>>> + '_ {
        let now = Utc::now();
        metadata
            .generations
            .into_iter()
            .sorted_by(|(_, metadata_1), (_, metadata_2)| {
                metadata_2.last_active.cmp(&metadata_1.last_active)
            })
            // don't gc DEFAULT_KEEP_GENERATIONS most recent generations
            .skip(DEFAULT_KEEP_GENERATIONS)
            .filter_map(move |(generation, metadata)| {
                let last_active = Utc
                    .timestamp_opt(metadata.last_active.try_into().unwrap(), 0)
                    .unwrap();
                let days_since_active = now.signed_duration_since(last_active).num_days();
                if days_since_active < 0 {
                    return Some(Err(DeleteSymlinksError::<Git>::TimestampInFuture(
                        generation,
                    )));
                }
                // current_gen must have been active more recently than any other
                // generation, so it will be skipped above
                if days_since_active > DEFAULT_MAX_AGE_DAYS.into() {
                    return Some(Ok(self.symlink_path(&generation)));
                }
                None
            })
    }
}

/// Implementations for R/O only instances
///
/// Mainly transformation into modifiable sandboxed instances
impl<'flox, Git: GitProvider> Environment<'flox, Git, ReadOnly<Git>> {
    /// Enter into editable mode by creating a git sandbox for the floxmeta
    pub async fn enter_transaction(
        self,
    ) -> Result<Environment<'flox, Git, GitSandBox<Git>>, TransactionEnterError<Git>> {
        let floxmeta = self.floxmeta.enter_transaction().await?;
        Ok(Environment {
            name: self.name,
            system: self.system,
            remote: self.remote,
            local: self.local,
            floxmeta,
        })
    }
}

/// Implementations for sandboxed only Environments
impl<'flox, Git: GitProvider> Environment<'flox, Git, GitSandBox<Git>> {
    /// Commit changes to environment by closing the underlying transaction
    pub async fn commit_transaction(
        self,
        message: &'flox str,
    ) -> Result<Environment<'_, Git, ReadOnly<Git>>, TransactionCommitError<Git>> {
        let floxmeta = self.floxmeta.commit_transaction(message).await?;
        Ok(Environment {
            name: self.name,
            system: self.system,
            remote: self.remote,
            local: self.local,
            floxmeta,
        })
    }
}

#[derive(Error, Debug)]
pub enum GetEnvironmentError<Git: GitProvider> {
    #[error("Environment not found")]
    NotFound,
    #[error(transparent)]
    GetEnvironment(#[from] GetEnvironmentsError<Git>),
    #[error(transparent)]
    GetFloxmeta(#[from] GetFloxmetaError<Git>),
}

#[derive(Error, Debug)]
pub enum GetEnvironmentsError<Git: GitProvider> {
    #[error("Failed listing environment branches: {0}")]
    ListBranches(Git::ListBranchesError),

    #[error("Failed fetching environment branches: {0}")]
    FetchBranches(Git::FetchError),
}

#[derive(Error, Debug)]
pub enum MetadataError<Git: GitProvider> {
    // todo: add environment name/path?
    #[error("Failed retrieving 'metadata.json': {0}")]
    RetrieveMetadata(Git::ShowError),

    #[error("Failed parsing 'metadata.json': {0}")]
    ParseMetadata(serde_json::Error),
}

#[derive(Error, Debug)]
pub enum CurrentGenerationError<Git: GitProvider> {
    #[error("Failed parsing 'metadata.json': {0}")]
    Generation(#[from] GenerationError<Git>),
}

#[derive(Error, Debug)]
pub enum GenerationError<Git: GitProvider> {
    #[error("Empty Environment")]
    Empty,

    #[error("Generation not found")]
    NotFound,

    #[error(transparent)]
    Metadata(#[from] MetadataError<Git>),

    #[error(transparent)]
    Manifest(#[from] ManifestError<Git>),
}

#[derive(Error, Debug)]
pub enum ManifestError<Git: GitProvider> {
    // todo: add environment name/path?
    #[error("Failed retrieving 'manifest.json': {0}")]
    RetrieveManifest(Git::ShowError),

    #[error("Failed parsing 'manifest.json': {0}")]
    ParseManifest(serde_json::Error),
}

#[derive(Error, Debug)]
pub enum DeleteSymlinksError<Git: GitProvider> {
    #[error(transparent)]
    Metadata(#[from] MetadataError<Git>),
    #[error("Found generation with last active timestamp in the future: {0}")]
    TimestampInFuture(String),
}

#[cfg(test)]
mod tests {

    use std::marker::PhantomData;

    use chrono::Days;

    use super::*;
    use crate::flox::tests::flox_instance;
    use crate::flox::Flox;
    use crate::providers::git::tests::mock_provider;
    use crate::providers::git::GitCommandProvider;

    fn mock_environment(
        flox: &Flox,
    ) -> Environment<'_, GitCommandProvider, ReadOnly<GitCommandProvider>> {
        let owner = "owner";
        let name = "name";
        let system = "system";

        let floxmeta = Floxmeta::<GitCommandProvider, ReadOnly<_>> {
            access: ReadOnly::new(mock_provider()),
            flox,
            owner: owner.to_string(),
            _git: PhantomData::default(),
        };

        Environment {
            floxmeta,
            local: None,
            name: name.to_string(),
            remote: None,
            system: system.to_string(),
        }
    }

    fn mock_generation_metadata(last_active: u64) -> GenerationMetadata {
        GenerationMetadata {
            created: 0,
            last_active,
            log_message: vec![],
            path: PathBuf::from("/does-not-exist"),
            version: 1,
        }
    }

    fn too_old() -> u64 {
        Utc::now()
            .checked_sub_days(Days::new((DEFAULT_MAX_AGE_DAYS + 1).into()))
            .unwrap()
            .timestamp() as u64
    }

    /// When there are DEFAULT_KEEP_GENERATIONS generations older than
    /// DEFAULT_MAX_AGE_DAYS, no generations are deleted.
    #[tokio::test]
    async fn symlinks_to_delete_keeps_generations() {
        let (flox, _tempdir_handle) = flox_instance();
        let environment = mock_environment(&flox);

        let mut generations = BTreeMap::new();
        let num_generations = DEFAULT_KEEP_GENERATIONS;
        for generation in 1..num_generations + 1 {
            generations.insert(
                generation.to_string(),
                mock_generation_metadata(too_old() - (num_generations - generation) as u64),
            );
        }

        let metadata = Metadata {
            current_gen: Some(num_generations.to_string()),
            generations,
            version: 1,
        };

        let mut symlinks = environment.symlinks_to_delete(metadata);
        assert!(symlinks.next().is_none());
    }

    /// When there are DEFAULT_KEEP_GENERATIONS+1 generations older than
    /// DEFAULT_MAX_AGE_DAYS, the oldest generation is deleted.
    #[tokio::test]
    async fn symlinks_to_delete_too_many_generations() {
        let (flox, _tempdir_handle) = flox_instance();
        let environment = mock_environment(&flox);

        let mut generations = BTreeMap::new();
        let num_generations = DEFAULT_KEEP_GENERATIONS + 1;
        for generation in 1..num_generations + 1 {
            generations.insert(
                generation.to_string(),
                mock_generation_metadata(too_old() - (num_generations - generation) as u64),
            );
        }

        let metadata = Metadata {
            current_gen: Some(num_generations.to_string()),
            generations,
            version: 1,
        };

        let mut symlinks = environment.symlinks_to_delete(metadata);
        assert_eq!(
            symlinks.next().unwrap().unwrap(),
            flox.data_dir.join("environments/owner/system.name-1-link")
        );
        assert!(symlinks.next().is_none());
    }

    /// When there are DEFAULT_KEEP_GENERATIONS+2 but all generations are as
    /// recent as DEFAULT_MAX_AGE_DAYS, no generations are deleted.
    #[tokio::test]
    async fn symlinks_to_delete_keeps_recent() {
        let (flox, _tempdir_handle) = flox_instance();
        let environment = mock_environment(&flox);

        let max_age_days_ago = Utc::now()
            .checked_sub_days(Days::new(DEFAULT_MAX_AGE_DAYS.into()))
            .unwrap()
            .timestamp();

        let mut generations = BTreeMap::new();
        let num_generations = DEFAULT_KEEP_GENERATIONS + 2;
        for generation in 1..num_generations + 1 {
            generations.insert(
                generation.to_string(),
                mock_generation_metadata(max_age_days_ago as u64 + generation as u64),
            );
        }

        let metadata = Metadata {
            current_gen: Some(num_generations.to_string()),
            generations,
            version: 1,
        };

        let mut symlinks = environment.symlinks_to_delete(metadata);
        assert!(symlinks.next().is_none());
    }

    /// When there are DEFAULT_KEEP_GENERATIONS+2 and every other generation
    /// is older than DEFAULT_MAX_AGE_DAYS, the two oldest generations are deleted.
    #[tokio::test]
    async fn symlinks_to_delete_deletes_oldest() {
        let (flox, _tempdir_handle) = flox_instance();
        let environment = mock_environment(&flox);

        let max_age_days_ago = Utc::now()
            .checked_sub_days(Days::new(DEFAULT_MAX_AGE_DAYS.into()))
            .unwrap()
            .timestamp();

        let mut generations = BTreeMap::new();
        let num_generations = DEFAULT_KEEP_GENERATIONS + 2;
        for generation in 1..num_generations + 1 {
            generations.insert(
                generation.to_string(),
                mock_generation_metadata(
                    // make even generations too old
                    if generation % 2 == 1 {
                        max_age_days_ago as u64 + generation as u64
                    } else {
                        too_old() - (num_generations - generation) as u64
                    },
                ),
            );
        }

        let metadata = Metadata {
            current_gen: Some(num_generations.to_string()),
            generations,
            version: 1,
        };

        let mut symlinks = environment.symlinks_to_delete(metadata);
        assert_eq!(
            symlinks.next().unwrap().unwrap(),
            flox.data_dir.join("environments/owner/system.name-4-link")
        );
        assert_eq!(
            symlinks.next().unwrap().unwrap(),
            flox.data_dir.join("environments/owner/system.name-2-link")
        );
        assert!(symlinks.next().is_none());
    }
}
