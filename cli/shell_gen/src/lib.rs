use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Unsupported shell '{}'", .0)]
    UnsupportedShell(String),
    #[error(transparent)]
    IO(#[from] std::io::Error),
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

    /// Get the name of the shell rather than the path to the executable
    pub fn name(&self) -> &str {
        match self {
            ShellWithPath::Bash(_) => "bash",
            ShellWithPath::Fish(_) => "fish",
            ShellWithPath::Tcsh(_) => "tcsh",
            ShellWithPath::Zsh(_) => "zsh",
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

pub trait GenerateShell {
    fn generate(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error>;
    fn to_stmt(&self) -> Statement;

    fn generate_with_newline(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        self.generate(shell, writer)?;
        writer.write_all("\n".as_bytes())?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SetVar {
    exported: bool,
    allow_expansion: bool,
    name: String,
    value: String,
}

impl SetVar {
    pub fn exported_no_expansion(name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            exported: true,
            allow_expansion: false,
            name: name.as_ref().to_string(),
            value: value.as_ref().to_string(),
        }
    }

    pub fn exported_with_expansion(name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            exported: true,
            allow_expansion: true,
            name: name.as_ref().to_string(),
            value: value.as_ref().to_string(),
        }
    }

    pub fn not_exported_no_expansion(name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            exported: false,
            allow_expansion: false,
            name: name.as_ref().to_string(),
            value: value.as_ref().to_string(),
        }
    }

    pub fn not_exported_with_expansion(name: impl AsRef<str>, value: impl AsRef<str>) -> Self {
        Self {
            exported: false,
            allow_expansion: true,
            name: name.as_ref().to_string(),
            value: value.as_ref().to_string(),
        }
    }

    pub fn unset(&self) -> UnsetVar {
        UnsetVar::new(self.name.clone())
    }
}

impl GenerateShell for SetVar {
    fn generate(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        match (shell, self.exported, self.allow_expansion) {
            (Shell::Bash, true, true) => {
                write!(writer, "export {}=\"{}\";", self.name, self.value).map_err(Error::IO)?;
            },
            (Shell::Bash, true, false) => {
                write!(writer, "export {}='{}\';", self.name, self.value).map_err(Error::IO)?;
            },
            (Shell::Bash, false, true) => {
                write!(writer, "{}=\"{}\";", self.name, self.value).map_err(Error::IO)?;
            },
            (Shell::Bash, false, false) => {
                write!(writer, "{}='{}\';", self.name, self.value).map_err(Error::IO)?;
            },
            (Shell::Zsh, true, true) => todo!(),
            (Shell::Zsh, true, false) => todo!(),
            (Shell::Zsh, false, true) => todo!(),
            (Shell::Zsh, false, false) => todo!(),
            (Shell::Tcsh, true, true) => todo!(),
            (Shell::Tcsh, true, false) => todo!(),
            (Shell::Tcsh, false, true) => todo!(),
            (Shell::Tcsh, false, false) => todo!(),
            (Shell::Fish, true, true) => todo!(),
            (Shell::Fish, true, false) => todo!(),
            (Shell::Fish, false, true) => todo!(),
            (Shell::Fish, false, false) => todo!(),
        }
        Ok(())
    }

    fn to_stmt(&self) -> Statement {
        Statement::SetVar(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct UnsetVar {
    name: String,
}

impl UnsetVar {
    pub fn new(name: impl AsRef<str>) -> Self {
        Self {
            name: name.as_ref().to_string(),
        }
    }
}

impl GenerateShell for UnsetVar {
    fn generate(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        match shell {
            Shell::Bash => {
                write!(writer, "unset {};", self.name).map_err(Error::IO)?;
            },
            Shell::Zsh => todo!(),
            Shell::Tcsh => todo!(),
            Shell::Fish => todo!(),
        }
        Ok(())
    }

    fn to_stmt(&self) -> Statement {
        Statement::UnsetVar(self.clone())
    }
}

#[derive(Debug, Clone)]
pub struct Source {
    path: PathBuf,
}

impl Source {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl GenerateShell for Source {
    fn generate(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        match shell {
            Shell::Bash => {
                write!(writer, "source '{}';", self.path.display()).map_err(Error::IO)?;
            },
            Shell::Zsh => todo!(),
            Shell::Tcsh => todo!(),
            Shell::Fish => todo!(),
        }
        Ok(())
    }

    fn to_stmt(&self) -> Statement {
        Statement::Source(self.clone())
    }
}

// So you can use the `generate` method on literal strings e.g. for things
// that aren't variable exports or sourcing files.
impl<T: AsRef<str>> GenerateShell for T {
    fn generate(&self, _shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        write!(writer, "{}", self.as_ref()).map_err(Error::IO)?;
        Ok(())
    }

    fn to_stmt(&self) -> Statement {
        Statement::Literal(self.as_ref().to_string())
    }
}

#[derive(Debug, Clone)]
pub enum Statement {
    SetVar(SetVar),
    UnsetVar(UnsetVar),
    Source(Source),
    Literal(String),
}

impl GenerateShell for Statement {
    fn generate(&self, shell: Shell, writer: &mut impl Write) -> Result<(), Error> {
        match self {
            Statement::SetVar(var) => var.generate(shell, writer),
            Statement::UnsetVar(var) => var.generate(shell, writer),
            Statement::Source(source) => source.generate(shell, writer),
            Statement::Literal(s) => s.generate(shell, writer),
        }
    }

    fn to_stmt(&self) -> Statement {
        // lol
        self.clone()
    }
}

pub fn set_unexported_unexpanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::not_exported_no_expansion(name, value).to_stmt()
}

pub fn set_exported_unexpanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::exported_no_expansion(name, value).to_stmt()
}

pub fn set_unexported_expanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::not_exported_with_expansion(name, value).to_stmt()
}

pub fn set_exported_expanded(name: impl AsRef<str>, value: impl AsRef<str>) -> Statement {
    SetVar::exported_with_expansion(name, value).to_stmt()
}

pub fn unset(name: impl AsRef<str>) -> Statement {
    UnsetVar::new(name).to_stmt()
}

pub fn source_file(path: impl AsRef<Path>) -> Statement {
    Source::new(path).to_stmt()
}

pub fn literal(s: impl AsRef<str>) -> Statement {
    Statement::Literal(s.as_ref().to_string())
}
