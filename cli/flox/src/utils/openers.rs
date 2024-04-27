use std::env;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};
use itertools::Itertools;
use log::debug;
use sysinfo::{Pid, System};

const OPENERS: &[&str] = &["xdg-open", "gnome-open", "kde-open"];

const BROWSER_OPENERS: &[&str] = &["www-browser"];

#[derive(Debug, Clone, PartialEq)]
pub struct Browser(PathBuf);
impl Browser {
    /// Find a system's "opener" command for the purpose of opening a browser.
    /// This is `xdg-open`, `gnome-open`, etc. on linux and `open` on macos
    ///
    /// In ssh sessions or TTYs without DISPLAY, a browser cannot be opened,
    /// so return an error.
    pub fn detect() -> Result<Self, String> {
        // in ssh sessions we can't open a browser
        if std::env::var("SSH_TTY").is_ok() {
            return Err("SSH session detected".into());
        }

        let browser = match std::env::consts::OS {
            "linux" => {
                // if X11 or wayland is not available, we can't open a browser
                if std::env::var("DISPLAY").is_err() && std::env::var("WAYLAND_DISPLAY").is_err() {
                    return Err("No X11 or Wayland display available".into());
                }
                let path_var =
                    env::var("PATH").map_err(|_| "Could not read PATH variable".to_string())?;
                let Some((path, executable)) = first_in_path(
                    [OPENERS, BROWSER_OPENERS].concat(),
                    env::split_paths(&path_var),
                ) else {
                    return Err("No opener found in PATH".to_string());
                };
                Self(path.join(executable))
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

#[derive(Debug, PartialEq, Clone)]
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
    pub fn detect_from_parent_process() -> Result<Self> {
        let parent_process_exe = get_parent_process_exe()?;

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
            Shell::Zsh(path) => path,
        }
    }
}

fn get_parent_process_exe() -> Result<PathBuf> {
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
    let parent_exe = parent_process
        .exe()
        .context("Failed to get parent process exe")?
        .to_path_buf();

    Ok(parent_exe)
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

#[cfg(test)]
mod tests {

    use super::*;

    /// On Linux, Browser::detect() finds xdg-open if it's in path
    ///
    /// TODO: we might want to better simulate an actual display and opener
    #[test]
    #[cfg(target_os = "linux")]
    fn test_browser_detect_finds_opener_in_path() {
        use std::fs::File;

        use tempfile::tempdir;

        let temp_dir = tempdir().unwrap();
        let xdg_open = temp_dir.path().join("xdg-open");
        File::create(&xdg_open).unwrap();

        temp_env::with_vars(
            [
                ("SSH_TTY", None),
                ("DISPLAY", Some("1")),
                (
                    "PATH",
                    Some(&format!(
                        "blah:blah:{}",
                        xdg_open.parent().unwrap().to_string_lossy()
                    )),
                ),
            ],
            || {
                assert_eq!(Browser::detect(), Ok(Browser(xdg_open)));
            },
        )
    }

    /// On macOS, Browser::detect() returns /usr/bin/open
    #[test]
    #[cfg(target_os = "macos")]
    fn test_browser_detect() {
        assert_eq!(
            Browser::detect(),
            Ok(Browser(PathBuf::from("/usr/bin/open")))
        );
    }

    /// Browser::detect() returns an error if SSH_TTY is set
    #[test]
    fn test_browser_detect_respects_ssh_tty() {
        temp_env::with_var("SSH_TTY", Some("1"), || {
            assert!(Browser::detect().is_err());
        });
    }

    /// Test the parsing of the shell from a path
    #[test]
    fn parse_shell() {
        let bash = PathBuf::from("/bin/bash");
        let zsh = PathBuf::from("/bin/zsh");

        assert_eq!(Shell::try_from(bash.as_path()).unwrap(), Shell::Bash(bash));
        assert_eq!(Shell::try_from(zsh.as_path()).unwrap(), Shell::Zsh(zsh));
        assert!(Shell::try_from(PathBuf::from("/bin/not_a_shell").as_path()).is_err())
    }

    /// Test the detection of the shell from the parent process
    #[test]
    fn test_get_parent_process_exe() {
        let path = get_parent_process_exe().expect("should find parent process");

        assert_eq!(path.file_name().unwrap(), "cargo");
    }

    /// Test the detection of the shell from environment variables
    #[test]
    fn test_detect_from_env_var() {
        temp_env::with_var("MYSHELL", Some("/bin/bash"), || {
            assert_eq!(
                Shell::detect_from_env("MYSHELL").unwrap(),
                Shell::Bash(PathBuf::from("/bin/bash"))
            );
        });

        temp_env::with_var_unset("MYSHELL", || {
            assert!(Shell::detect_from_env("MYSHELL").is_err());
        });
    }
}
