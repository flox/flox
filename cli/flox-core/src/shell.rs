use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, System};
use tracing::debug;

#[derive(Debug, PartialEq, Clone, Serialize, Deserialize)]
pub enum Shell {
    Bash(PathBuf),
    Fish(PathBuf),
    Tcsh(PathBuf),
    Zsh(PathBuf),
}

impl TryFrom<&Path> for Shell {
    type Error = anyhow::Error;

    fn try_from(value: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.file_name() {
            Some(name) if name == "bash" => Ok(Shell::Bash(value.to_owned())),
            Some(name) if name == "fish" => Ok(Shell::Fish(value.to_owned())),
            Some(name) if name == "tcsh" => Ok(Shell::Tcsh(value.to_owned())),
            Some(name) if name == "zsh" => Ok(Shell::Zsh(value.to_owned())),
            _ => Err(anyhow!("Unsupported shell {value:?}")),
        }
    }
}

impl Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Shell::Bash(_) => write!(f, "bash"),
            Shell::Fish(_) => write!(f, "fish"),
            Shell::Tcsh(_) => write!(f, "tcsh"),
            Shell::Zsh(_) => write!(f, "zsh"),
        }
    }
}

impl Shell {
    /// Detect the current shell from the parent process
    ///
    /// This function tries to detect the shell from the parent process.
    /// If reading process information of the parent process fails,
    /// or the exe path of the parent process can not be parsed to a known shell,
    /// an error is returned.
    pub fn detect_from_parent_process() -> Result<Self> {
        let parent_process_exe = get_parent_process_exe()?;
        debug!("Detected parent process exe: {parent_process_exe:?}");

        Self::try_from(parent_process_exe.as_path())
    }

    /// Detect the current shell from the {var} environment variable
    pub fn detect_from_env(var: &str) -> Result<Self> {
        env::var(var)
            .with_context(|| format!("{var} environment variable not set"))
            .and_then(|shell| {
                let path = PathBuf::from(shell);
                Self::try_from(path.as_path())
            })
    }

    /// Get the path to the shell executable
    pub fn exe_path(&self) -> &Path {
        match self {
            Shell::Bash(path) => path,
            Shell::Fish(path) => path,
            Shell::Tcsh(path) => path,
            Shell::Zsh(path) => path,
        }
    }
}

fn get_parent_process_exe() -> Result<PathBuf> {
    let parent_pid = Pid::from_u32(std::os::unix::process::parent_id());
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::Some(&[parent_pid]), false);

    let parent_process = system
        .process(parent_pid)
        .context("Failed to get info about parent process")?;

    // Investigate whether to use `parent_process.cmd()[0]` instead.
    // Shells often have a compatibility mode with `sh` if invoked as `sh`.
    // The current approach will only pick this mode up if the filename is sh e.g.
    // symlinked to bash or zsh.
    // Using `argv[0]` may still be unreliable as a path to a shell executable,
    // if set manually by the calling process or the parent shell itself.
    //
    // However, all this is only relevant once we want to detect more shells
    // -- including `sh` -- and not just `bash` and `zsh`.
    let parent_exe = parent_process
        .exe()
        .context("Failed to get parent process exe")?
        .to_path_buf();

    Ok(parent_exe)
}
