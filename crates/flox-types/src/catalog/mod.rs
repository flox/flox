use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json;
use serde_with::skip_serializing_none;

use crate::version::Version;

mod build;
pub use build::Build;
pub mod cache;
pub use cache::Cache;
mod element;
pub use element::Element;
mod eval;
pub use eval::Eval;
mod source;
pub use source::Source;

pub type DerivationPath = PathBuf;
pub type StorePath = PathBuf;
pub type AttrPath = Vec<String>;
pub type PackageVersion = String;
pub type Stability = String;
/// TODO use runix FlakeRef
pub type FlakeRef = String;

#[skip_serializing_none]
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub build: Option<Build>,
    pub cache: Option<Cache>,
    pub element: Element,
    pub eval: Eval,
    pub version: Version<1>,
    pub source: Option<Source>,
    #[serde(rename = "type")]
    pub type_: Option<Type>,
}

/// type for Nix
#[derive(Serialize, Deserialize)]
pub enum Type {
    #[serde(rename = "catalogRender")]
    CatalogRender,
}

#[derive(Serialize, Deserialize)]
pub struct StabilityCatalog(BTreeMap<Stability, BTreeMap<PackageVersion, CatalogEntry>>);

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
        let raw_contents = fs::read_to_string(&json_path).unwrap();
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
        let raw_contents = fs::read_to_string(&json_path).unwrap();
        let catalog_entry: CatalogEntry = serde_json::from_str(&raw_contents).unwrap();
        let serialized = serde_json::to_string_pretty(&catalog_entry).unwrap();
        let raw_value: Value = serde_json::from_str(&raw_contents).unwrap();
        let serialized_value: Value = serde_json::from_str(&serialized).unwrap();
        assert_eq!(raw_value, serialized_value);
    }
}
