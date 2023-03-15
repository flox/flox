use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use log::debug;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::floxmeta::Floxmeta;
use crate::providers::git::{BranchInfo, GitProvider};

#[derive(Serialize, Debug)]
pub struct Environment<'flox, G> {
    name: String,
    system: String,
    remote: Option<EnvBranch>,
    local: Option<EnvBranch>,
    #[serde(skip)]
    floxmeta: &'flox Floxmeta<'flox, G>,
}

#[derive(Serialize, Debug)]
#[allow(unused)]
pub struct EnvBranch {
    description: String,
    hash: String,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub current_gen: String,
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
impl<Git: GitProvider> Floxmeta<'_, Git> {
    pub async fn environment(
        &self,
        name: &str,
    ) -> Result<Environment<Git>, GetEnvironmentError<Git>> {
        self.environments()
            .await?
            .into_iter()
            .find(|env| env.name == name && env.system == self.flox.system)
            .ok_or(GetEnvironmentError::NotFound)
    }

    /// Detect all environments from a floxmeta repo
    pub async fn environments(&self) -> Result<Vec<Environment<Git>>, GetEnvironmentsError<Git>> {
        self.git
            .fetch()
            .await
            .map_err(GetEnvironmentsError::FetchBranches)?;

        // get output of `git branch -av`
        let list_branches_output = self
            .git
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
                    floxmeta: self,
                }
            })
            .fold(
                HashMap::new(),
                |mut merged: HashMap<_, Environment<_>>, mut env| {
                    // inplace either remote or local in a reference we already stored
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
impl<Git: GitProvider> Environment<'_, Git> {
    pub async fn metadata(&self) -> Result<Metadata, MetadataError<Git>> {
        let git = &self.floxmeta.git;
        let metadata_str = git
            .show(&format!(
                "{}.{}:{}",
                self.system, self.name, "metadata.json"
            ))
            .await
            .map_err(MetadataError::RetrieveMetadata)?;

        let metadata: Metadata = serde_json::from_str(&metadata_str.to_string_lossy())
            .map_err(MetadataError::ParseMetadata)?;

        Ok(metadata)
    }

    pub async fn generation(&self, generation: &str) -> Result<Generation, GenerationError<Git>> {
        let git = &self.floxmeta.git;
        let mut metadata = self.metadata().await?;
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
}

#[derive(Error, Debug)]
pub enum GetEnvironmentError<Git: GitProvider> {
    #[error("Environment not found")]
    NotFound,
    #[error(transparent)]
    GetEnvironment(#[from] GetEnvironmentsError<Git>),
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
pub enum GenerationError<Git: GitProvider> {
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
