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

pub fn source_file(path: &Path) -> String {
    format!("source '{}';", path.display())
}
