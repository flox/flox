use std::path::{Path, PathBuf};

use anyhow::bail;
use clap::Args;
use log::debug;

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
        let existing_dirs = separate_dir_list(&self.env_dirs);
        let new_dirs = populate_env_dirs(&self.flox_env, &existing_dirs);
        let joined = join_dir_list(new_dirs);
        let sourceable_commands = match self.shell.as_ref() {
            "bash" => {
                format!("export FLOX_ENV_DIRS=\"{joined}\";")
            },
            "zsh" => {
                format!("export FLOX_ENV_DIRS=\"{joined}\";")
            },
            "fish" => {
                format!("set -gx FLOX_ENV_DIRS \"{joined}\";")
            },
            "tcsh" => {
                format!("setenv FLOX_ENV_DIRS \"{joined}\";")
            },
            other => {
                bail!("invalid shell: {other}")
            },
        };
        output.write_all(sourceable_commands.as_bytes())?;
        debug!("Set FLOX_ENV_DIRS, FLOX_ENV_DIRS={}", joined);
        Ok(())
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
            shell: "bash".to_string(),
        };
        let mut buf = Vec::new();
        args.handle_inner(&mut buf).unwrap();
        let buf = String::from_utf8_lossy(&buf);
        let expected = format!("export FLOX_ENV_DIRS=\"{flox_env}:{env_dirs}\";");
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
}
