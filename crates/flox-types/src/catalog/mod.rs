use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{self, Value};
use serde_with::skip_serializing_none;
use thiserror::Error;

use crate::stability::Stability;
use crate::version::Version;

mod build;
pub use build::Build;
pub mod cache;
pub use cache::Cache;
mod element;
pub use element::{Element, PublishElement};
mod eval;
pub use eval::Eval;
mod source;
pub use source::Source;

pub type DerivationPath = PathBuf;
pub type StorePath = PathBuf;
pub type AttrPath = Vec<String>;
/// The "meaningful" component of an AttrPath. This excludes derivation type,
/// channel, system, stability, and version
///
/// TODO https://docs.rs/nonempty/latest/nonempty/
pub type Namespace = AttrPath;
pub type PackageVersion = String;
pub type System = String;
/// TODO use runix FlakeRef
pub type FlakeRef = String;

pub type ChannelRef = String;

#[skip_serializing_none]
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub build: Option<Build>,
    pub cache: Option<Cache>,
    pub element: Element,
    pub eval: Eval,
    /// TODO deprecate
    pub publish_element: Option<Value>,
    pub version: Version<1>,
    pub source: Option<Source>,
    #[serde(rename = "type")]
    pub type_: Option<Type>,
}

/// type for Nix
#[derive(Clone, Serialize, Deserialize)]
pub enum Type {
    #[serde(rename = "catalogRender")]
    CatalogRender,
}

#[derive(Serialize, Deserialize)]
pub struct StabilityCatalog(BTreeMap<Stability, BTreeMap<PackageVersion, CatalogEntry>>);

/// Type that more closely mirrors the JSON structure for ease of
/// seralization/deserialization
type RawEnvCatalog =
    BTreeMap<ChannelRef, BTreeMap<System, BTreeMap<Stability, BTreeMap<String, PathOrEntry>>>>;

/// A collection of publishes of publishes
#[derive(Clone, Deserialize)]
#[serde(from = "RawEnvCatalog")]
pub struct EnvCatalog {
    pub entries: BTreeMap<PublishElement, CatalogEntry>,
}

impl From<RawEnvCatalog> for EnvCatalog {
    fn from(value: RawEnvCatalog) -> Self {
        let entries = value
            .into_iter()
            .flat_map(|(channel, other)| {
                other
                    .into_iter()
                    .map(move |(system, other)| (channel.clone(), system, other))
            })
            .flat_map(|(channel, system, other)| {
                other.into_iter().map(move |(stability, other)| {
                    (channel.clone(), system.clone(), stability, other)
                })
            })
            .flat_map(|(ref channel, ref system, ref stability, other)| {
                PathOrEntry::Path(other)
                    .entries()
                    .into_iter()
                    .flat_map(move |(namespace, entry)| {
                        entry.into_iter().map(move |(version, entry)| {
                            (
                                PublishElement {
                                    namespace: namespace.clone(),
                                    // TODO normalize to include flake:
                                    // That will probably happen once
                                    // FlakeRef is an actual FlakeRef rather
                                    // than String
                                    original_url: channel.clone(),
                                    stability: stability.clone(),
                                    system: system.clone(),
                                    version,
                                },
                                entry,
                            )
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .collect();

        EnvCatalog { entries }
    }
}

#[derive(Error, Debug)]
pub enum SerializeEnvCatalogError {
    #[error("Found zero length namespace")]
    EmptyAttrSubPath,
    #[error("Cannot insert {0} as a subpackage of another package")]
    SubpackageOfPackage(String),
    #[error("Cannot insert {0} as a package because it is already a package set")]
    ExistingPackageSet(String),
    #[error("Package version {0} already exists")]
    VersionExists(PackageVersion),
}

impl Serialize for EnvCatalog {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        // don't use RawEnvCatalog because we want to use &String
        let mut raw_env_catalog = BTreeMap::new();

        for (publish_element, catalog_entry) in &self.entries {
            // Get channel.system.stability
            let namespace_map = raw_env_catalog
                .entry(&publish_element.original_url)
                .or_insert(BTreeMap::new())
                .entry(&publish_element.system)
                .or_insert(BTreeMap::new())
                .entry(&publish_element.stability)
                .or_insert(BTreeMap::new());

            let (first, all_but_first) =
                publish_element.namespace.split_first().ok_or_else(|| {
                    serde::ser::Error::custom(SerializeEnvCatalogError::EmptyAttrSubPath)
                })?;

            let mut path_or_entry = namespace_map
                    .entry(first)
                    // If namespace has length 1, we need to insert an entry
                    // because the for loop below will be skipped
                    .or_insert(if all_but_first.is_empty() {
                        PathOrEntry::Entry(BTreeMap::new())
                    } else {
                        PathOrEntry::Path(BTreeMap::new())
                    });

            // Loop for the rest of namespace
            // will iterate at least once, `break`ing immidiately
            let mut path = all_but_first.iter().peekable();
            while let Some(segment) = path.next() {
                match path_or_entry {
                    PathOrEntry::Entry(_) => {
                        Err(serde::ser::Error::custom(
                            SerializeEnvCatalogError::SubpackageOfPackage(segment.clone()),
                        ))?;
                    },
                    PathOrEntry::Path(p) => {
                        let next_segment = match path.peek() {
                            Some(_) => PathOrEntry::Path(BTreeMap::new()),
                            None => PathOrEntry::Entry(BTreeMap::new()),
                        };
                        path_or_entry = p.entry(segment.to_string()).or_insert(next_segment);
                    },
                }
            }

            let versions = match path_or_entry {
                // we cant place a package inside the attrpath of another package
                // i.e. with `foo.bar.<version> = <entry>` we cant have an entry `foo.<version> = <entry>`
                PathOrEntry::Path(_) => Err(serde::ser::Error::custom(
                    SerializeEnvCatalogError::ExistingPackageSet(
                        publish_element.namespace.last().unwrap().clone(),
                    ),
                ))?,
                PathOrEntry::Entry(versions) => versions,
            };

            if versions
                .insert(publish_element.version.clone(), catalog_entry.clone())
                .is_some()
            {
                Err(serde::ser::Error::custom(
                    SerializeEnvCatalogError::VersionExists(publish_element.version.clone()),
                ))?
            }
        }

        raw_env_catalog.serialize(serializer)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathOrEntry {
    Entry(BTreeMap<PackageVersion, CatalogEntry>),
    Path(BTreeMap<String, PathOrEntry>),
}

impl PathOrEntry {
    pub fn entries(self) -> BTreeMap<Vec<String>, BTreeMap<PackageVersion, CatalogEntry>> {
        match self {
            PathOrEntry::Path(path_and_other) => path_and_other
                .into_iter()
                .flat_map(|(path_element, path_or_entry)| {
                    path_or_entry
                        .entries()
                        .into_iter()
                        .map(|(mut path, element)| {
                            path.insert(0, path_element.clone());
                            (path, element)
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .collect(),
            PathOrEntry::Entry(version_and_entry) => {
                BTreeMap::from_iter([(Vec::new(), version_and_entry)])
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;
    use serde_json::Value;

    use super::*;

    #[test]
    fn update_catalog_file() {
        let json_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("curl-handcrafted.json");
        let raw_contents = fs::read_to_string(json_path).unwrap();
        let stability_catalog: StabilityCatalog = serde_json::from_str(&raw_contents).unwrap();
        let serialized = serde_json::to_string(&stability_catalog).unwrap();
        let raw_value: Value = serde_json::from_str(&raw_contents).unwrap();
        let serialized_value: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(raw_value, serialized_value);
    }
    #[test]
    fn flox_publish_file() {
        let json_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("flox-handcrafted.json");
        let raw_contents = fs::read_to_string(json_path).unwrap();
        let catalog_entry: CatalogEntry = serde_json::from_str(&raw_contents).unwrap();
        let serialized = serde_json::to_string_pretty(&catalog_entry).unwrap();
        let raw_value: Value = serde_json::from_str(&raw_contents).unwrap();
        let serialized_value: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(raw_value, serialized_value);
    }

    #[test]
    fn flox_env_catalog() {
        for filename in ["env-catalog.json", "env-catalog-nested.json"] {
            let json_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests")
                .join(filename);
            let raw_contents = fs::read_to_string(json_path).unwrap();
            let catalog: EnvCatalog = serde_json::from_str(&raw_contents).unwrap();
            let serialized = serde_json::to_string_pretty(&catalog).unwrap();
            let raw_value: Value = serde_json::from_str(&raw_contents).unwrap();
            let serialized_value: Value = serde_json::from_str(&serialized).unwrap();
            assert_eq!(raw_value, serialized_value);
        }
    }
}
