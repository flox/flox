/// Rust abstraction over the nix command line
/// Candidate for a standalone library to build arbitrary Nix commands in a safe manner
use anyhow::Result;
use arguments::NixArgs;
use async_trait::async_trait;

pub mod arguments;
pub mod command;
pub mod command_line;
pub mod installable;

pub use command_line as default;

/// Abstract nix interface
///
/// Runs a command as described as [NixArgs] by the `args` parameter.
/// Implementing methods for each "nix command" is pointless
/// as the implementation can be more cleanly abstracted to sets of possible configuration.
/// The sets are modeled after their implementation in Nix.
///
/// Future extensions of this trait may include running with text/json/rnix deserialization
#[async_trait]
pub trait NixApi {
    /// passthru nix
    async fn run(&self, args: NixArgs) -> Result<()>;
}

trait MergeArgs {
    /// Merge with another NixCommonArgs instance in-place
    /// Useful to override/extend previouly globally set variables
    fn merge(&mut self, other: &Self) -> Result<()>;
}
