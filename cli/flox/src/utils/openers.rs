use std::env;
use std::path::PathBuf;
use std::process::Command;

use anyhow::Result;
use itertools::Itertools;
use tracing::debug;

const OPENERS: &[&str] = &["xdg-open", "gnome-open", "kde-open"];

const BROWSER_OPENERS: &[&str] = &["www-browser"];

#[derive(Debug, Clone, PartialEq)]
pub enum Browser {
    BrowserCommand(PathBuf, Vec<String>),
    GenericOpener(PathBuf),
}
impl Browser {
    /// Create a new Browser instance from a command and arguments
    /// as defined in the BROWSER environment variable.
    ///
    /// If unset or empty, find a system's "opener" command
    /// for the purpose of opening a browser.
    /// This is `xdg-open`, `gnome-open`, etc. on linux and `open` on macos.
    /// When using an opener in ssh sessions or TTYs without DISPLAY,
    /// a browser cannot practically be opened, return an error in that case.
    pub fn detect() -> Result<Self, String> {
        // If the BROWSER environment variable is set, use that
        if let Some(browser) = Self::detect_by_browser_var() {
            return Ok(browser);
        }

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
                Self::GenericOpener(path.join(executable))
            },
            "macos" => Self::GenericOpener(PathBuf::from("/usr/bin/open")),
            unsupported => {
                debug!("Unsupported OS '{unsupported}' cannot open a browser");
                return Err(format!("Unsupported OS '{unsupported}'"));
            },
        };

        debug!("Detected browser opener: {browser:?}");
        Ok(browser)
    }

    fn detect_by_browser_var() -> Option<Self> {
        let Ok(browser_var) = env::var("BROWSER") else {
            debug!("BROWSER environment variable not set");
            return None;
        };

        match browser_var.split_whitespace().collect_vec()[..] {
            [] => {
                debug!("BROWSER environment variable is empty");
                None
            },
            [command, ref args @ ..] => {
                let command = PathBuf::from(command);
                let args = args.iter().map(|s| s.to_string()).collect();
                let browser = Self::BrowserCommand(command, args);
                Some(browser)
            },
        }
    }

    pub fn to_command(&self) -> Command {
        match self {
            Browser::BrowserCommand(executable, arguments) => {
                let mut command = Command::new(executable);
                command.args(arguments);
                command
            },
            Browser::GenericOpener(executable) => Command::new(executable),
        }
    }
}

pub fn first_in_path<'a, I>(
    candidates: I,
    path: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, &'a str)>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: Clone,
{
    path.into_iter()
        .cartesian_product(candidates)
        .find(|(path, candidate)| path.join(candidate).exists())
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
                ("BROWSER", None),
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
                assert_eq!(Browser::detect(), Ok(Browser::GenericOpener(xdg_open)));
            },
        )
    }

    /// On macOS, Browser::detect() returns /usr/bin/open
    #[test]
    #[cfg(target_os = "macos")]
    fn test_browser_detect() {
        temp_env::with_var_unset("BROWSER", || {
            assert_eq!(
                Browser::detect(),
                Ok(Browser::GenericOpener(PathBuf::from("/usr/bin/open")))
            );
        })
    }

    /// Browser::detect() returns an error if SSH_TTY is set
    #[test]
    fn test_browser_detect_respects_ssh_tty() {
        temp_env::with_vars([("SSH_TTY", Some("1")), ("BROWSER", None)], || {
            assert!(Browser::detect().is_err());
        });
    }

    /// Browser::detect() the value of BROWSER environment variable if set
    #[test]
    fn test_browser_detect_by_browser_var() {
        let browser = "firefox -P my-profile";
        temp_env::with_var("BROWSER", Some(browser), || {
            assert_eq!(
                Browser::detect(),
                Ok(Browser::BrowserCommand(PathBuf::from("firefox"), vec![
                    "-P".to_string(),
                    "my-profile".to_string()
                ]))
            );
        });
    }

    /// Test the parsing of the shell from a path
    #[test]
    fn parse_shell() {
        let bash = PathBuf::from("/bin/bash");
        let fish = PathBuf::from("/bin/fish");
        let tcsh = PathBuf::from("/bin/tcsh");
        let zsh = PathBuf::from("/bin/zsh");

        assert_eq!(Shell::try_from(bash.as_path()).unwrap(), Shell::Bash(bash));
        assert_eq!(Shell::try_from(fish.as_path()).unwrap(), Shell::Fish(fish));
        assert_eq!(Shell::try_from(tcsh.as_path()).unwrap(), Shell::Tcsh(tcsh));
        assert_eq!(Shell::try_from(zsh.as_path()).unwrap(), Shell::Zsh(zsh));
        assert!(Shell::try_from(PathBuf::from("/bin/not_a_shell").as_path()).is_err())
    }

    /// Test the detection of the shell from the parent process
    #[test]
    fn test_get_parent_process_exe() {
        let path = get_parent_process_exe().expect("should find parent process");

        let parent = path.file_name().unwrap();
        assert!(parent == "cargo" || parent == "cargo-nextest");
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
