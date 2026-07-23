//! Generated OpenAPI client for the Factory Service API.
//!
//! The client is generated from the OpenAPI spec in `openapi.json` using the
//! `progenitor` crate. The spec is a 3.0.2-converted snapshot of the Factory
//! Service's runtime `app.openapi()` output. It is refreshed by running
//! `just factory openapi-export` in the floxhub repository and copying the
//! output here.
//!
//! `src/client.rs` is generated and checked in — regenerate it by running
//! `cargo build -p factory-api-v1` after updating `openapi.json`.

mod client;
pub mod hooks;
mod status;
pub use client::*;
pub use hooks::RequestHooks;

pub mod types {
    pub use crate::client::types::*;
    pub use crate::status::EffectiveBuildStatus;
}
