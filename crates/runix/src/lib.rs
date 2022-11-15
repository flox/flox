use std::error::Error;

/// Rust abstraction over the nix command line
/// Candidate for a standalone library to build arbitrary Nix commands in a safe manner
use arguments::NixArgs;
use async_trait::async_trait;

pub mod arguments;
pub mod command;
pub mod command_line;
pub mod installable;

pub use command_line as default;

pub trait NixBackend {}

#[async_trait]
pub trait Run<B: NixBackend> {
    type Error: 'static + Error + Send + Sync;
    async fn run(&self, backend: &B, nix_args: &NixArgs) -> Result<(), Self::Error>;
}

#[async_trait]
pub trait RunJson<B: NixBackend>: Run<B> {
    async fn json(&self, backend: &B, nix_args: &NixArgs) -> Result<(), Self::Error>;
}
