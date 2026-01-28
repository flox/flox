use std::fmt::Display;

use serde::{Deserialize, Serialize};

mod nix;
mod nix_build_config;
mod nix_build_lock;

/// Common identifier for a `CatalogSpec` and its `CatalogLock`
/// within a build config and lock respectively.
/// Also exposed to nix expressions in the NEF as the `<catalog>` in
/// ```nix
/// {catalogs}:
/// let
///    catalogs.<catalog>.<package>
/// in ...
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct CatalogId(String);

impl Display for CatalogId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub use nix_build_config::{lock_config, read_config};
pub use nix_build_lock::write_lock;
