//! In-memory model of an installed extension.

use std::path::PathBuf;

use super::manifest::InstalledState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Extension {
    pub name: String,
    pub install_dir: PathBuf,
    pub state: InstalledState,
}
