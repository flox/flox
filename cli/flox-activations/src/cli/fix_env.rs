use std::path::PathBuf;

use anyhow::bail;
use clap::Args;
use tracing::debug;

use super::fix_paths::{fix_manpath_var, fix_path_var};
use super::set_env_dirs::fix_env_dirs_var;

/// Combined command that sets FLOX_ENV_DIRS, PATH, and MANPATH in one call.
/// This replaces separate `set-env-dirs` + `fix-paths` invocations in
/// command mode, eliminating one process spawn.
#[derive(Debug, Args)]
pub struct FixEnvArgs {
    #[arg(short = 'f', long, help = "The contents of FLOX_ENV.")]
    pub flox_env: PathBuf,
    #[arg(help = "Which shell syntax to return.")]
    #[arg(short, long, value_name = "SHELL")]
    pub shell: String,
    #[arg(help = "The contents of FLOX_ENV_DIRS.")]
    #[arg(short, long, value_name = "STRING")]
    pub env_dirs: String,
    #[arg(help = "The contents of PATH.")]
    #[arg(short, long, value_name = "STRING")]
    pub path: String,
    #[arg(help = "The contents of MANPATH.")]
    #[arg(short, long, value_name = "STRING")]
    pub manpath: String,
}

impl FixEnvArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut stdout = std::io::stdout();
        self.handle_inner(&mut stdout)
    }

    fn handle_inner(&self, output: &mut impl std::io::Write) -> Result<(), anyhow::Error> {
        debug!(
            "Preparing to fix env, FLOX_ENV={}, FLOX_ENV_DIRS={}",
            self.flox_env.display(),
            self.env_dirs
        );

        // Step 1: compute new FLOX_ENV_DIRS
        let new_env_dirs = fix_env_dirs_var(&self.flox_env, &self.env_dirs);

        // Step 2: compute new PATH and MANPATH using the updated env dirs
        let new_path = fix_path_var(&new_env_dirs, &self.path);
        let new_manpath = fix_manpath_var(&new_env_dirs, &self.manpath);

        let sourceable_commands = match self.shell.as_ref() {
            "bash" | "zsh" => format!(
                "export FLOX_ENV_DIRS=\"{new_env_dirs}\";\nexport PATH=\"{new_path}\";\nexport MANPATH=\"{new_manpath}\";\n"
            ),
            "fish" => format!(
                "set -gx FLOX_ENV_DIRS \"{new_env_dirs}\";\nset -gx PATH \"{new_path}\";\nset -gx MANPATH \"{new_manpath}\";\n"
            ),
            "tcsh" => format!(
                "setenv FLOX_ENV_DIRS \"{new_env_dirs}\";\nsetenv PATH \"{new_path}\";\nsetenv MANPATH \"{new_manpath}\";\n"
            ),
            other => bail!("invalid shell: {other}"),
        };

        output.write_all(sourceable_commands.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::io::BufRead;

    use super::*;

    #[test]
    fn combines_set_env_dirs_and_fix_paths() {
        let mut buf = vec![];
        FixEnvArgs {
            flox_env: PathBuf::from("/env1"),
            shell: "bash".to_string(),
            env_dirs: "".to_string(),
            path: "/usr/bin".to_string(),
            manpath: "/usr/share/man".to_string(),
        }
        .handle_inner(&mut buf)
        .unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("FLOX_ENV_DIRS="));
        assert!(output.contains("PATH="));
        assert!(output.contains("MANPATH="));
    }

    #[test]
    fn lines_have_trailing_semicolons() {
        let shells = ["bash", "zsh", "fish", "tcsh"];
        for shell in shells.iter() {
            let mut buf = vec![];
            FixEnvArgs {
                flox_env: PathBuf::from("/env1"),
                shell: shell.to_string(),
                env_dirs: "/env2".to_string(),
                path: "/usr/bin".to_string(),
                manpath: "/usr/share/man".to_string(),
            }
            .handle_inner(&mut buf)
            .unwrap();
            for line in buf.lines() {
                assert!(line.unwrap().ends_with(';'));
            }
        }
    }
}
