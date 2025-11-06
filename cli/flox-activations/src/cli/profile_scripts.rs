use std::path::Path;

use clap::Args;
use log::debug;

use super::{join_dir_list, separate_dir_list};
use crate::shell_gen::{Shell, source_file};

#[derive(Debug, Args)]
pub struct ProfileScriptsArgs {
    #[arg(
        short,
        long,
        help = "The contents of the FLOX_ENV_DIRS variable (which may be unset or empty)."
    )]
    pub env_dirs: String,
    #[arg(short, long, help = "Which shell syntax to emit")]
    pub shell: Shell,
    #[arg(
        long,
        help = "A list of FLOX_ENV directories that have already been sourced"
    )]
    // If not provided, defaults to an empty string. This is required for tcsh in
    // particular that is unable to pass empty arguments on the command line.
    #[clap(default_value = "")]
    pub already_sourced_env_dirs: String,
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
        let individual_cmds = source_profile_scripts_cmds(
            &self.env_dirs,
            &self.already_sourced_env_dirs,
            &self.shell,
            Path::exists,
        );
        let all_cmds = individual_cmds.join(";\n");
        output.write_all(all_cmds.as_bytes())?;
        Ok(())
    }
}

/// Returns a list of commands for sourcing the user's profile scripts in the correct order and updating _FLOX_SOURCED_PROFILE_SCRIPTS.
///
/// The `path_predicate` is used to determine which paths should actually be sourced. In production
/// this is just any of the profile scripts that exist on disk, but in testing it can be anything.
/// This is mostly useful so that you don't need to specifically create files to test this functionality.
fn source_profile_scripts_cmds(
    env_dirs: &str,
    already_sourced_env_dirs: &str,
    shell: &Shell,
    path_predicate: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let dirs = separate_dir_list(env_dirs);
    let already_sourced_dirs = separate_dir_list(already_sourced_env_dirs);
    let mut new_sourced_dirs = already_sourced_dirs.clone();
    let mut commands = Vec::new();
    for dir in dirs.into_iter().rev() {
        if !already_sourced_dirs.contains(&dir) {
            let common = dir.join("activate.d/profile-common");
            let shell_specific = dir.join(format!("activate.d/profile-{shell}"));
            for path in [common, shell_specific] {
                if path_predicate(&path) {
                    commands.push(source_file(&path));
                } else {
                    debug!("script did not exist: {}", path.display());
                }
            }
            new_sourced_dirs.insert(0, dir)
        }
    }
    commands.push(shell.set_var_not_exported(
        "_FLOX_SOURCED_PROFILE_SCRIPTS",
        &join_dir_list(new_sourced_dirs),
    ));
    commands.push("".to_string()); // ensure there's a trailing newline when joining
    commands
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn bash_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Bash, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-bash'".to_string(),
            "source 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-bash'".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Zsh, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-zsh'".to_string(),
            "source 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-zsh'".to_string(),
            "typeset -g _FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Tcsh, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-tcsh'".to_string(),
            "source 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-tcsh'".to_string(),
            "set _FLOX_SOURCED_PROFILE_SCRIPTS = 'newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Fish, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-fish'".to_string(),
            "source 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-fish'".to_string(),
            "set -g _FLOX_SOURCED_PROFILE_SCRIPTS 'newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Bash, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-bash'".to_string(),
            "source 'newer/activate.d/profile-bash'".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Zsh, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-zsh'".to_string(),
            "source 'newer/activate.d/profile-zsh'".to_string(),
            "typeset -g _FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Tcsh, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-tcsh'".to_string(),
            "source 'newer/activate.d/profile-tcsh'".to_string(),
            "set _FLOX_SOURCED_PROFILE_SCRIPTS = 'newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, "", &Shell::Fish, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common'".to_string(),
            "source 'older/activate.d/profile-fish'".to_string(),
            "source 'newer/activate.d/profile-fish'".to_string(),
            "set -g _FLOX_SOURCED_PROFILE_SCRIPTS 'newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn one_already_sourced_script_is_skipped() {
        let dirs = "newer:older";
        let already_sourced = "older";
        let cmds = source_profile_scripts_cmds(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source 'newer/activate.d/profile-common'".to_string(),
            "source 'newer/activate.d/profile-bash'".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn all_already_sourced_scripts_are_skipped() {
        let dirs = "newer:older";
        let already_sourced = "newer:older";
        let cmds = source_profile_scripts_cmds(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec!["_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older'".to_string(), "".to_string()];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn prepend_already_sourced_when_no_overlap() {
        let dirs = "standalone";
        let already_sourced = "already:existing";
        let cmds = source_profile_scripts_cmds(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source 'standalone/activate.d/profile-common'".to_string(),
            "source 'standalone/activate.d/profile-bash'".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='standalone:already:existing'".to_string(),
	    "".to_string(),
        ];
        assert_eq!(expected, cmds);
    }
}
