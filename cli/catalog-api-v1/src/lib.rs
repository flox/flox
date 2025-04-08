//! This module contains the generated OpenAPI client for the Catalog API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the `progenitor` crate.
//! The spec is managed by the Catalog API team and is updated when the upstream API changes.

mod client;
mod error;
pub use client::*;

#[cfg(any(test, feature = "tests"))]
#[allow(clippy::all)]
pub mod mock;

pub mod types {
    pub use crate::client::types::*;
    pub use crate::error::MessageType;

    use serde::{Deserialize, Serialize};
    /// Progenitor doesn't know how to use a discriminator as a tag, so add this
    /// enum manually.
    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    #[serde(tag = "store_type", rename_all = "kebab-case")]
    pub enum CatalogStoreConfig {
        /// The catalog store has not yet been configured
        Null,
        /// The user has configured the catalog for metadata only publishes
        MetaOnly,
        /// Store to copy to with `nix copy`
        NixCopy(CatalogStoreConfigNixCopy),
        /// Not yet supported
        Publisher,
    }

    #[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
    pub struct CatalogStoreConfigNixCopy {
        pub egress_uri: String,
        pub ingress_uri: String,
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{CatalogStoreConfig, CatalogStoreConfigNixCopy};

    #[test]
    fn deserialize_catalog_store_config_null() {
        let response_string = r#"{
            "store_type": "null"
        }"#;

        let store_config = serde_json::from_str::<CatalogStoreConfig>(response_string).unwrap();
        assert_eq!(store_config, CatalogStoreConfig::Null)
    }

    #[test]
    fn deserialize_catalog_store_config_meta_only() {
        let response_string = r#"{
            "store_type": "meta-only"
        }"#;

        let store_config = serde_json::from_str::<CatalogStoreConfig>(response_string).unwrap();
        assert_eq!(store_config, CatalogStoreConfig::MetaOnly)
    }

    #[test]
    fn deserialize_catalog_store_config_nix_copy() {
        let response_string = r#"{
           "store_type": "nix-copy",
           "ingress_uri": "s3://example",
           "egress_uri": "s3://example"
        }"#;

        let store_config = serde_json::from_str::<CatalogStoreConfig>(response_string).unwrap();
        assert_eq!(
            store_config,
            CatalogStoreConfig::NixCopy(CatalogStoreConfigNixCopy {
                ingress_uri: "s3://example".into(),
                egress_uri: "s3://example".into()
            })
        )
    }

    #[test]
    fn deserialize_catalog_store_config_publisher() {
        let response_string = r#"{
           "store_type": "publisher"
        }"#;

        let store_config = serde_json::from_str::<CatalogStoreConfig>(response_string).unwrap();
        assert_eq!(store_config, CatalogStoreConfig::Publisher)
    }
}
