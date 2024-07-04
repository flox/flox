//! This module contains the generated OpenAPI client for the Catalog API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the `progenitor` crate.
//! The spec is managed by the Catalog API team and is updated when the upstream API changes.

mod client;
mod error;
pub use client::*;

pub mod types {
    pub use crate::error::MessageType;
    pub use crate::client::types::*;
}
