//! This module contains the generated OpenAPI client for the Catalog API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the `progenitor` crate.
//! The spec is managed by the Catalog API team and is updated when the upstream API changes.

mod client;
mod error;
pub use client::*;

pub mod types {
    pub use crate::client::types::*;
    pub use crate::error::MessageType;

    use serde::{Deserialize, Serialize};
    /// Progenitor doesn't know how to use a discriminator as a tag, so add this
    /// enum manually.
    ///
    /// We still embed the underlying variant types, which have an extraneous
    /// `store_type` field, so that we don't shadow changes in the catalog API.
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
        Publisher(CatalogStoreConfigPublisher),
    }
}

#[cfg(test)]
mod tests {
    use crate::types::{
        CatalogStoreConfig, CatalogStoreConfigNixCopy, CatalogStoreConfigPublisher,
    };

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
                egress_uri: "s3://example".into(),
                store_type: "nix-copy".into(),
            })
        )
    }

    #[test]
    fn deserialize_catalog_store_config_publisher() {
        let response_string = r#"{
           "store_type": "publisher",
           "publisher_url": "s3://example"
        }"#;

        let store_config = serde_json::from_str::<CatalogStoreConfig>(response_string).unwrap();
        assert_eq!(
            store_config,
            CatalogStoreConfig::Publisher(CatalogStoreConfigPublisher {
                publisher_url: Some("s3://example".to_string()),
                store_type: "publisher".into(),
            })
        )
    }
}
