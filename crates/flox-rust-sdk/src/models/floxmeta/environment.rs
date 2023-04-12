use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use log::debug;
use runix::installable::Installable;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::{Floxmeta, GetFloxmetaError, TransactionCommitError, TransactionEnterError};
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

#[derive(Serialize, Deserialize, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub current_gen: Option<String>,
    generations: BTreeMap<String, GenerationMetadata>,
    #[serde(default)]
    version: u32,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GenerationMetadata {
    created: u64,
    last_active: u64,
    log_message: Vec<String>,
    path: PathBuf,
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
    pub fn name(&self) -> Cow<str> {
        Cow::from(&self.name)
    }

    pub fn system(&self) -> Cow<str> {
        Cow::from(&self.system)
    }

    pub fn remote(&self) -> Cow<Option<EnvBranch>> {
        Cow::Borrowed(&self.remote)
    }

    pub fn local(&self) -> Cow<Option<EnvBranch>> {
        Cow::Borrowed(&self.local)
    }

    pub fn owner(&self) -> &str {
        self.floxmeta.owner()
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

        Ok(Installable::new(
            // todo: replace with flakeref
            format!(
                "git+path://{root}?ref={system}.{name}&dir={generation}",
                root = git.path().to_string_lossy(),
                system = self.system(),
                name = self.name()
            ),
            ".floxEnvs.default".to_string(),
        ))
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
