use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::Args;
use tracing::debug;

use super::{join_dir_list, separate_dir_list};

#[derive(Debug, Args)]
pub struct SetEnvDirsArgs {
    #[arg(short, long, help = "The contents of the FLOX_ENV variable")]
    pub flox_env: PathBuf,
    #[arg(
        short,
        long,
        help = "The contents of the FLOX_ENV_DIRS variable (which may be unset or empty)."
    )]
    pub env_dirs: String,
    #[arg(
        long,
        default_value = "",
        help = "The contents of the _FLOX_ENV_DIRS_ADD_SBIN variable (which may be unset or empty)."
    )]
    pub sbin_dirs: String,
    #[arg(
        long,
        help = "Whether to add the environment's sbin directory to PATH."
    )]
    pub add_sbin: bool,
    #[arg(short, long, help = "Which shell syntax to emit")]
    pub shell: String,
}

impl SetEnvDirsArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut output = std::io::stdout();
        self.handle_inner(&mut output)
    }

    fn handle_inner(&self, output: &mut impl std::io::Write) -> Result<(), anyhow::Error> {
        debug!(
            "Preparing to set FLOX_ENV_DIRS, FLOX_ENV={}, FLOX_ENV_DIRS={}",
            self.flox_env.display(),
            self.env_dirs
        );
        let new_env_dirs = fix_env_dirs_var(&self.flox_env, &self.env_dirs);
        let new_sbin_dirs = fix_sbin_dirs_var(&self.flox_env, self.add_sbin, &self.sbin_dirs);
        let sourceable_commands = match self.shell.as_ref() {
            "bash" | "zsh" => {
                format!(
                    "export FLOX_ENV_DIRS=\"{new_env_dirs}\";\nexport _FLOX_ENV_DIRS_ADD_SBIN=\"{new_sbin_dirs}\";"
                )
            },
            "fish" => {
                format!(
                    "set -gx FLOX_ENV_DIRS \"{new_env_dirs}\";\nset -gx _FLOX_ENV_DIRS_ADD_SBIN \"{new_sbin_dirs}\";"
                )
            },
            "tcsh" => {
                format!(
                    "setenv FLOX_ENV_DIRS \"{new_env_dirs}\";\nsetenv _FLOX_ENV_DIRS_ADD_SBIN \"{new_sbin_dirs}\";"
                )
            },
            other => {
                bail!("invalid shell: {other}")
            },
        };
        output.write_all(sourceable_commands.as_bytes())?;
        debug!(
            "Set FLOX_ENV_DIRS, FLOX_ENV_DIRS={}, _FLOX_ENV_DIRS_ADD_SBIN={}",
            new_env_dirs, new_sbin_dirs
        );
        Ok(())
    }
}

pub fn fix_env_dirs_var(flox_env: impl AsRef<Path>, env_dirs: impl AsRef<str>) -> String {
    let existing_dirs = separate_dir_list(env_dirs.as_ref());
    let new_dirs = populate_env_dirs(flox_env.as_ref(), &existing_dirs);
    join_dir_list(new_dirs)
}

/// Calculate the new _FLOX_ENV_DIRS_ADD_SBIN value: the subset of
/// FLOX_ENV_DIRS whose environments want their `sbin` directory on PATH.
/// If `add_sbin` is set, the environment is prepended to the list (unless
/// already present); otherwise the list is passed through unchanged.
///
/// An empty list is represented by the "empty" sentinel (understood by
/// `separate_dir_list`) rather than an empty string: tcsh versions before
/// 6.23 drop an empty `$var:q` argument from the command line entirely,
/// which would make `--sbin-dirs` consume the next argument as its value.
pub fn fix_sbin_dirs_var(
    flox_env: impl AsRef<Path>,
    add_sbin: bool,
    sbin_dirs: impl AsRef<str>,
) -> String {
    let existing_dirs = separate_dir_list(sbin_dirs.as_ref());
    let new_dirs = if add_sbin {
        populate_env_dirs(flox_env.as_ref(), &existing_dirs)
    } else {
        existing_dirs
    };
    let joined = join_dir_list(new_dirs);
    if joined.is_empty() {
        "empty".to_string()
    } else {
        joined
    }
}

/// Adds a new environment to the list of active env dirs if its not already present.
fn populate_env_dirs(flox_env: &Path, env_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut new_dirs = vec![];
    let flox_env = flox_env.to_path_buf();
    if !env_dirs.contains(&flox_env) {
        new_dirs.push(flox_env);
        new_dirs.extend_from_slice(env_dirs);
    } else {
        new_dirs.extend_from_slice(env_dirs)
    }
    new_dirs
}

#[cfg(test)]
mod test {
    use std::io::BufRead;

    use super::*;

    #[test]
    fn skips_adding_duplicate_flox_env() {
        let flox_env = PathBuf::from("/foo/bar");
        let env_dirs = [
            PathBuf::from("/baz"),
            PathBuf::from("/qux"),
            PathBuf::from("/foo/bar"),
        ];
        let new_dirs = populate_env_dirs(&flox_env, &env_dirs);
        assert_eq!(new_dirs, env_dirs);
    }

    #[test]
    fn prepends_to_existing_dirs() {
        let env_dirs = "/bar:/baz".to_string();
        let flox_env = "/foo";
        let args = SetEnvDirsArgs {
            flox_env: PathBuf::from(flox_env),
            env_dirs: env_dirs.clone(),
            sbin_dirs: String::new(),
            add_sbin: false,
            shell: "bash".to_string(),
        };
        let mut buf = Vec::new();
        args.handle_inner(&mut buf).unwrap();
        let buf = String::from_utf8_lossy(&buf);
        let expected = format!(
            "export FLOX_ENV_DIRS=\"{flox_env}:{env_dirs}\";\nexport _FLOX_ENV_DIRS_ADD_SBIN=\"empty\";"
        );
        assert_eq!(buf, expected);
    }

    #[test]
    fn prepends_to_sbin_dirs_when_add_sbin() {
        let args = SetEnvDirsArgs {
            flox_env: PathBuf::from("/foo"),
            env_dirs: "/bar".to_string(),
            sbin_dirs: "/bar".to_string(),
            add_sbin: true,
            shell: "bash".to_string(),
        };
        let mut buf = Vec::new();
        args.handle_inner(&mut buf).unwrap();
        let buf = String::from_utf8_lossy(&buf);
        let expected =
            "export FLOX_ENV_DIRS=\"/foo:/bar\";\nexport _FLOX_ENV_DIRS_ADD_SBIN=\"/foo:/bar\";";
        assert_eq!(buf, expected);
    }

    #[test]
    fn lines_have_trailing_semicolons() {
        let shells = ["bash", "zsh", "fish", "tcsh"];
        let env_dirs = "/env1:/env2";
        let flox_env = "/foo/bar";
        for shell in shells.iter() {
            let mut buf = vec![];
            SetEnvDirsArgs {
                flox_env: PathBuf::from(flox_env),
                env_dirs: env_dirs.to_string(),
                sbin_dirs: String::new(),
                add_sbin: true,
                shell: shell.to_string(),
            }
            .handle_inner(&mut buf)
            .unwrap();
            for line in buf.lines() {
                assert!(line.unwrap().ends_with(';'));
            }
        }
    }

    #[test]
    fn handles_empty_env_dirs() {
        let flox_env = PathBuf::from("/foo");
        let env_dirs = vec![];
        let new_dirs = populate_env_dirs(&flox_env, &env_dirs);
        assert_eq!(new_dirs, vec![flox_env]);
    }

    #[test]
    fn sbin_dirs_prepended_when_enabled() {
        let fixed = fix_sbin_dirs_var("/foo", true, "/bar:/baz");
        assert_eq!(fixed, "/foo:/bar:/baz");
    }

    #[test]
    fn sbin_dirs_not_duplicated() {
        let fixed = fix_sbin_dirs_var("/foo", true, "/foo:/bar");
        assert_eq!(fixed, "/foo:/bar");
    }

    #[test]
    fn sbin_dirs_unchanged_when_disabled() {
        let fixed = fix_sbin_dirs_var("/foo", false, "/bar:/baz");
        assert_eq!(fixed, "/bar:/baz");
    }

    #[test]
    fn sbin_dirs_sentinel_when_disabled_and_empty() {
        let fixed = fix_sbin_dirs_var("/foo", false, "");
        assert_eq!(fixed, "empty");
    }

    #[test]
    fn sbin_dirs_handles_empty_sentinel() {
        let fixed = fix_sbin_dirs_var("/foo", true, "empty");
        assert_eq!(fixed, "/foo");
    }
}
