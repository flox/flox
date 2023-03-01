use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::floxmeta::Floxmeta;
use super::Root;
use crate::providers::git::GitProvider;

pub struct Environment<'flox, G> {
    name: String,
    system: String,
    floxmeta: &'flox Floxmeta<G>,
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
impl<Git: GitProvider> Root<'_, Floxmeta<Git>> {
    /// Add a new flox style package from a template.
    /// Uses `nix flake init` to retrieve files
    /// and postprocesses the generic templates.
    pub async fn environment(
        &self,
        name: &str,
    ) -> Result<Root<'_, Environment<Git>>, GetEnvironmentError>
where {
        Ok(Root {
            flox: self.flox,
            state: Environment {
                name: name.to_string(),
                system: self.flox.system.clone(),
                floxmeta: &self.state,
            },
        })
    }
}

/// Implementations for an environment
impl<Git: GitProvider> Root<'_, Environment<'_, Git>> {
    pub async fn metadata(&self) -> Result<Metadata, MetadataError<Git>> {
        let git = &self.state.floxmeta.git;
        let metadata_str = git
            .show(&format!(
                "{}.{}:{}",
                self.state.system, self.state.name, "metadata.json"
            ))
            .await
            .map_err(MetadataError::RetrieveMetadata)?;

        let metadata: Metadata = serde_json::from_str(&metadata_str.to_string_lossy())
            .map_err(MetadataError::ParseMetadata)?;

        Ok(metadata)
    }

    pub async fn generation(&self, generation: &str) -> Result<Generation, GenerationError<Git>> {
        let git = &self.state.floxmeta.git;
        let mut metadata = self.metadata().await?;
        let generation_metadata = metadata
            .generations
            .remove(generation)
            .ok_or(GenerationError::NotFound)?;
        let manifest_content = git
            .show(&format!(
                "{}.{}:{}/{}",
                self.state.system, self.state.name, generation, "manifest.json"
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
pub enum GetEnvironmentError {}

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
