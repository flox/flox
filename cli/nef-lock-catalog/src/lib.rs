use std::fmt::Display;

use serde::{Deserialize, Serialize};

mod lock;
mod nix;
mod nix_build_config;
mod scan;

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

pub use lock::build_lock::{BuildLock, write_lock};
pub use lock::flakeref::NixFlakeref;
pub use lock::lookup::{LockError, lock_references};
pub use lock::render::render_unresolvable;
pub use nix_build_config::{LockOptions, lock_config, lock_config_with_options, read_config};
pub use scan::{CatalogRef, scan_package, scan_package_with_roots};
