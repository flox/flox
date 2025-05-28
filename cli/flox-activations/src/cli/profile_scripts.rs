use std::path::Path;

use clap::Args;
use log::debug;

use super::separate_dir_list;
use crate::shell_gen::source_file;

#[derive(Debug, Args)]
pub struct ProfileScriptsArgs {
    #[arg(
        short,
        long,
        help = "The contents of the FLOX_ENV_DIRS variable (which may be unset or empty)."
    )]
    pub env_dirs: String,
    #[arg(short, long, help = "Which shell syntax to emit")]
    pub shell: String,
}

impl ProfileScriptsArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut output = std::io::stdout();
        self.handle_inner(&mut output)
    }

    fn handle_inner(&self, output: &mut impl std::io::Write) -> Result<(), anyhow::Error> {
        debug!(
            "producing profile script commands, FLOX_ENV_DIRS={}",
            self.env_dirs
        );
        let individual_cmds =
            source_profile_scripts_cmds(&self.env_dirs, &self.shell, Path::exists);
        output.write_all(individual_cmds.join("\n").as_bytes())?;
        Ok(())
    }
}

/// Returns a list of commands for sourcing the user's profile scripts in the correct order.
///
/// The output also contains comment lines indicating any errors or files that didn't exist.
/// The `path_predicate` is used to determine which paths should actually be sourced. In production
/// this is just any of the profile scripts that exist on disk, but in testing it can be anything.
/// This is mostly useful so that you don't need to specifically create files to test this functionality.
fn source_profile_scripts_cmds(
    env_dirs: &str,
    shell: &str,
    path_predicate: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let dirs = separate_dir_list(env_dirs);
    dirs.into_iter()
        .rev()
        .flat_map(|path| {
            let common = path.join("activate.d/profile-common");
            let shell_specific = path.join(format!("activate.d/profile-{shell}"));
            [common, shell_specific]
        })
        .map(|path| {
            if path_predicate(&path) {
                source_file(&path)
            } else {
                format!("# Script did not exist: '{}'", path.display())
            }
        })
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn bash_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "bash", |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-bash';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-bash';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "zsh", |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-zsh';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-zsh';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "tcsh", |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-tcsh';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-tcsh';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "fish", |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-fish';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-fish';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "bash", |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-bash';".to_string(),
            "# Script did not exist: 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-bash';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "zsh", |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-zsh';".to_string(),
            "# Script did not exist: 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-zsh';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "tcsh", |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-tcsh';".to_string(),
            "# Script did not exist: 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-tcsh';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "fish", |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-fish';".to_string(),
            "# Script did not exist: 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-fish';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }
}
