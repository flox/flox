use std::fmt::Display;
use std::path::{Path, PathBuf};

use anyhow::anyhow;
use serde::{Deserialize, Serialize};

/// ShellWithPath represents a shell along with a PathBuf used to run it,
/// although the PathBuf may or may not be absolute
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum ShellWithPath {
    Bash(PathBuf),
    Fish(PathBuf),
    Tcsh(PathBuf),
    Zsh(PathBuf),
}

impl TryFrom<&Path> for ShellWithPath {
    type Error = anyhow::Error;

    fn try_from(value: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.file_name() {
            Some(name) if name == "bash" => Ok(ShellWithPath::Bash(value.to_owned())),
            Some(name) if name == "fish" => Ok(ShellWithPath::Fish(value.to_owned())),
            Some(name) if name == "tcsh" => Ok(ShellWithPath::Tcsh(value.to_owned())),
            Some(name) if name == "zsh" => Ok(ShellWithPath::Zsh(value.to_owned())),
            _ => Err(anyhow!("Unsupported shell {value:?}")),
        }
    }
}

impl Display for ShellWithPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShellWithPath::Bash(_) => write!(f, "bash"),
            ShellWithPath::Fish(_) => write!(f, "fish"),
            ShellWithPath::Tcsh(_) => write!(f, "tcsh"),
            ShellWithPath::Zsh(_) => write!(f, "zsh"),
        }
    }
}

impl ShellWithPath {
    /// Get the path to the shell executable
    pub fn exe_path(&self) -> &Path {
        match self {
            ShellWithPath::Bash(path) => path,
            ShellWithPath::Fish(path) => path,
            ShellWithPath::Tcsh(path) => path,
            ShellWithPath::Zsh(path) => path,
        }
    }
}
