use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use itertools::Itertools;
use log::{debug, warn};
use sysinfo::{Pid, System};

const OPENERS: &[&str] = &["xdg-open", "gnome-open", "kde-open"];

const BROWSER_OPENERS: &[&str] = &["www-browser"];

#[derive(Debug, Clone)]
pub struct Browser(PathBuf);
impl Browser {
    /// Open a url in the default browser using the system's "opener" command
    /// This is `xdg-open`, `gnome-open`, etc. on linux and `open` on macos
    ///
    /// In ssh sessions or TTYs without DISPLAY, a browser cannot be opened
    pub fn detect() -> Result<Self, String> {
        // in ssh sessions we can't open a browser
        if std::env::var("SSH_TTY").is_ok() {
            return Err("SSH session detected".into());
        }

        // if X11 or wayland is not available, we can't open a browser
        if std::env::consts::OS == "linux"
            && std::env::var("DISPLAY").is_err()
            && std::env::var("WAYLAND_DISPLAY").is_err()
        {
            return Err("No X11 or Wayland display available".into());
        }

        let browser = match std::env::consts::OS {
            "linux" => {
                let path_var =
                    env::var("PATH").map_err(|_| "Could not read PATH variable".to_string())?;
                let Some((path, _)) = first_in_path(
                    [OPENERS, BROWSER_OPENERS].concat(),
                    env::split_paths(&path_var),
                ) else {
                    return Err("No opener found in PATH".to_string());
                };
                Self(path)
            },
            "macos" => Self(PathBuf::from("/usr/bin/open")),
            unsupported => {
                debug!("Unsupported OS '{unsupported}' cannot open a browser");
                return Err(format!(
                    "Unsupported OS '{unsupported}'",
                    unsupported = unsupported
                ));
            },
        };

        debug!("Detected browser opener: {browser:?}");
        Ok(browser)
    }

    #[allow(unused)]
    pub fn path(&self) -> &Path {
        &self.0
    }

    #[allow(unused)]
    pub fn name(&self) -> String {
        self.0.file_name().unwrap().to_string_lossy().into_owned()
    }

    pub fn to_command(&self) -> Command {
        Command::new(&self.0)
    }
}

#[derive(Debug)]
pub enum Shell {
    Bash(PathBuf),
    Zsh(PathBuf),
}

impl TryFrom<&Path> for Shell {
    type Error = anyhow::Error;

    fn try_from(value: &Path) -> std::prelude::v1::Result<Self, Self::Error> {
        match value.file_name() {
            Some(name) if name == "bash" => Ok(Shell::Bash(value.to_owned())),
            Some(name) if name == "zsh" => Ok(Shell::Zsh(value.to_owned())),
            _ => Err(anyhow!("Unsupported shell {value:?}")),
        }
    }
}

impl Display for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Shell::Bash(_) => write!(f, "bash"),
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
    fn detect_from_parent_process() -> Result<Self> {
        // todo: we can narrow down the amount of data collected by sysinfo, for now collect everything
        let system = System::new_all();

        let parent_process = system
            .process(Pid::from_u32(std::os::unix::process::parent_id()))
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
        let parent_exe: &Path = parent_process
            .exe()
            .context("Failed to get parent process exe")?;

        Self::try_from(parent_exe)
    }

    /// Detect the current shell from the {var} environment variable
    fn detect_from_env(var: &str) -> Result<Self> {
        env::var(var)
            .with_context(|| format!("{var} environment variable not set"))
            .and_then(|shell| {
                let path = PathBuf::from(shell);
                Self::try_from(path.as_path())
            })
    }

    /// Detect the executing shell
    ///
    /// This function first tries to detect the shell from the parent process,
    /// and falls back to the SHELL environment variable if that fails.
    /// If both fail, an error is returned.
    ///
    /// Both methods are overridable by setting the FLOX_SHELL environment variable.
    pub fn detect() -> Result<Self> {
        Self::detect_from_env("FLOX_SHELL")
            .or_else(|_| Self::detect_from_parent_process())
            .or_else(|err| {
                warn!("Failed to detect shell from parent process: {err}");

                Self::detect_from_env("SHELL")
            })
    }

    /// Get the path to the shell executable
    pub fn exe_path(&self) -> &Path {
        match self {
            Shell::Bash(path) => path,
            Shell::Zsh(path) => path,
        }
    }
}

fn first_in_path<'a, I>(
    candidates: I,
    path: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, &'a str)>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: Clone,
{
    path.into_iter()
        .cartesian_product(candidates)
        .find(|(path, editor)| path.join(editor).exists())
}
