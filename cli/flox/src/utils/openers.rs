use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use itertools::Itertools;
use log::debug;

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
                let Some((path, _)) = first_in_path([OPENERS, BROWSER_OPENERS].concat(), env::split_paths(&path_var)) else { return Err("No opener found in PATH".to_string()) };
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

fn first_in_path<'a, I>(
    candidates: I,
    path: impl IntoIterator<Item = PathBuf>,
) -> Option<(PathBuf, &'a str)>
where
    I: IntoIterator<Item = &'a str>,
    I::IntoIter: Clone,
{
    path.into_iter()
        .cartesian_product(candidates.into_iter())
        .find(|(path, editor)| path.join(editor).exists())
}
