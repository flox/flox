use std::fmt::Display;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unsupported shell '{}'", .0)]
    UnsupportedShell(String),
}

/// ShellWithPath represents a shell along with a PathBuf used to run it,
/// although the PathBuf may or may not be absolute
#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShellWithPath {
    Bash(PathBuf),
    Fish(PathBuf),
    Tcsh(PathBuf),
    Zsh(PathBuf),
}

impl TryFrom<&Path> for ShellWithPath {
    type Error = Error;

    fn try_from(value: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.file_name() {
            Some(name) if name == "bash" => Ok(ShellWithPath::Bash(value.to_owned())),
            Some(name) if name == "fish" => Ok(ShellWithPath::Fish(value.to_owned())),
            Some(name) if name == "tcsh" => Ok(ShellWithPath::Tcsh(value.to_owned())),
            Some(name) if name == "zsh" => Ok(ShellWithPath::Zsh(value.to_owned())),
            _ => Err(Error::UnsupportedShell(value.to_string_lossy().to_string())),
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

/// The shells that we support generating code for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Tcsh,
    Fish,
}

impl std::str::FromStr for Shell {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "tcsh" => Ok(Self::Tcsh),
            "fish" => Ok(Self::Fish),
            _ => Err(Error::UnsupportedShell(s.to_string())),
        }
    }
}

impl From<ShellWithPath> for Shell {
    fn from(value: ShellWithPath) -> Self {
        match value {
            ShellWithPath::Bash(_) => Shell::Bash,
            ShellWithPath::Fish(_) => Shell::Fish,
            ShellWithPath::Tcsh(_) => Shell::Tcsh,
            ShellWithPath::Zsh(_) => Shell::Zsh,
        }
    }
}

impl Display for Shell {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::Bash => write!(f, "bash"),
            Self::Zsh => write!(f, "zsh"),
            Self::Tcsh => write!(f, "tcsh"),
            Self::Fish => write!(f, "fish"),
        }
    }
}

impl Shell {
    /// Set a shell variable that is not exported
    pub fn set_var_not_exported(&self, var: &str, value: &str) -> String {
        match self {
            Self::Bash => format!("{var}='{value}';"),
            Self::Fish => format!("set -g {var} '{value}';"),
            Self::Tcsh => format!("set {var} = '{value}';"),
            Self::Zsh => format!("typeset -g {var}='{value}';"),
        }
    }
}

pub fn source_file(path: &Path) -> String {
    format!("source '{}';", path.display())
}
