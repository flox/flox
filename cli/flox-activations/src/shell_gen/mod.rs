pub mod bash;
pub mod capture;

use std::fmt;
use std::path::Path;

/// The shells that we support generating code for
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Tcsh,
    Fish,
}

impl std::str::FromStr for Shell {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "bash" => Ok(Self::Bash),
            "zsh" => Ok(Self::Zsh),
            "tcsh" => Ok(Self::Tcsh),
            "fish" => Ok(Self::Fish),
            _ => Err(anyhow::anyhow!("Invalid shell: '{s}'")),
        }
    }
}

impl fmt::Display for Shell {
    // This trait requires `fmt` with this exact signature.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
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
    /// Include a trailing ;
    pub fn set_var_not_exported(&self, var: &str, value: &str) -> String {
        match self {
            Self::Bash => format!("{var}='{value}';"),
            Self::Fish => format!("set -g {var} '{value}';"),
            Self::Tcsh => format!("set {var} = '{value}';"),
            Self::Zsh => format!("typeset -g {var}='{value}';"),
        }
    }

    /// Do not include a trailing ;
    pub fn export_var(&self, var: &str, value: &str) -> String {
        match self {
            Self::Bash => format!("export {var}='{value}'"),
            _ => unimplemented!(),
        }
    }
}

pub fn source_file(path: &Path) -> String {
    format!("source '{}';", path.display())
}
