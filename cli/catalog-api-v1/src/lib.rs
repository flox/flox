//! This module contains the generated OpenAPI client for the Catalog API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the `progenitor` crate.
//! The spec is managed by the Catalog API team and is updated when the upstream API changes.

include!(concat!(env!("OUT_DIR"), "/client.rs"));

/// A mock server for the api based on the OpenAPI spec.
pub mod mock {
    include!(concat!(env!("OUT_DIR"), "/mock.rs"));
}
