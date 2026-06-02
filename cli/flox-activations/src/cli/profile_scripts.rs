use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Args;
use shell_gen::Shell;
use tracing::debug;

use super::{join_dir_list, separate_dir_list};

/// Name of the runtime variable that tracks which env dirs have already
/// had their profile scripts sourced. Read by the activate/deactivate
/// snippets to skip env dirs that are already covered (stacked envs).
const SOURCED_PROFILE_SCRIPTS_ENV: &str = "_FLOX_SOURCED_PROFILE_SCRIPTS";

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
        debug!(
            env_dirs = %self.env_dirs,
            "producing profile script commands"
        );
        let individual_cmds = source_profile_scripts_cmds_activate(
            &self.env_dirs,
            &self.already_sourced_env_dirs,
            &self.shell,
            Path::exists,
        );
        let all_cmds = format!("{}\n", individual_cmds.join("\n"));
        std::io::stdout().write_all(all_cmds.as_bytes())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Args)]
pub struct ProfileScriptsDeactivateArgs {
    #[arg(
        short,
        long,
        help = "Single env directory being deactivated (typically $FLOX_ENV)."
    )]
    pub env: PathBuf,
    #[arg(short, long, help = "Which shell syntax to emit")]
    pub shell: Shell,
    #[arg(
        long,
        help = "Current value of _FLOX_SOURCED_PROFILE_SCRIPTS; --env will be removed from it"
    )]
    // Empty default matches `ProfileScriptsArgs::already_sourced_env_dirs` so
    // tcsh can pass an unset variable without quoting gymnastics.
    #[clap(default_value = "")]
    pub already_sourced_env_dirs: String,
}

impl ProfileScriptsDeactivateArgs {
    pub fn handle(&self) -> Result<(), anyhow::Error> {
        debug!(
            env = %self.env.display(),
            "producing profile deactivate commands"
        );
        let individual_cmds = source_profile_scripts_cmds_deactivate(
            &self.env,
            &self.already_sourced_env_dirs,
            &self.shell,
            Path::exists,
        );
        let all_cmds = format!("{}\n", individual_cmds.join("\n"));
        std::io::stdout().write_all(all_cmds.as_bytes())?;
        Ok(())
    }
}

/// Returns a list of commands for sourcing the user's activation profile
/// scripts in the correct order (`activate.d/profile-common` then
/// `activate.d/profile-{shell}` per env) and updating
/// `_FLOX_SOURCED_PROFILE_SCRIPTS` with the env dirs that were sourced.
///
/// The `path_predicate` is used to determine which paths should actually be
/// sourced. In production this is just any of the profile scripts that exist
/// on disk, but in testing it can be anything. This is mostly useful so that
/// you don't need to specifically create files to test this functionality.
fn source_profile_scripts_cmds_activate(
    env_dirs: &str,
    already_sourced_env_dirs: &str,
    shell: &Shell,
    path_predicate: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let dirs = separate_dir_list(env_dirs);
    let already_sourced_dirs = separate_dir_list(already_sourced_env_dirs);
    let mut new_sourced_dirs = already_sourced_dirs.clone();
    let mut cmds = vec![];
    for dir in dirs.into_iter().rev() {
        if !already_sourced_dirs.contains(&dir) {
            for path in [
                dir.join("activate.d/profile-common"),
                dir.join(format!("activate.d/profile-{shell}")),
            ] {
                if path_predicate(&path) {
                    cmds.push(format!("source '{}';", path.display()));
                } else {
                    debug!(path = %path.display(), "script did not exist");
                }
            }
            new_sourced_dirs.insert(0, dir)
        }
    }
    cmds.push(shell.set_var_not_exported(
        SOURCED_PROFILE_SCRIPTS_ENV,
        &join_dir_list(new_sourced_dirs),
    ));
    cmds
}

/// Returns a list of commands for sourcing the user's deactivation profile
/// scripts in inverse order (`activate.d/deactivate-profile-{shell}` then
/// `activate.d/deactivate-profile-common` — LIFO cleanup so per-shell
/// teardown runs before common teardown), then updating
/// `_FLOX_SOURCED_PROFILE_SCRIPTS` to remove `env` from the list. The
/// tracking-var update is unconditional: the env is being torn down whether
/// or not it had any deactivate scripts.
///
/// The `path_predicate` is used to determine which paths should actually be
/// sourced; see `source_profile_scripts_cmds_activate` for the same pattern.
fn source_profile_scripts_cmds_deactivate(
    env: &Path,
    already_sourced_env_dirs: &str,
    shell: &Shell,
    path_predicate: impl Fn(&Path) -> bool,
) -> Vec<String> {
    let mut cmds = vec![];
    for path in [
        env.join(format!("activate.d/deactivate-profile-{shell}")),
        env.join("activate.d/deactivate-profile-common"),
    ] {
        if path_predicate(&path) {
            cmds.push(format!("source '{}';", path.display()));
        } else {
            debug!(path = %path.display(), "script did not exist");
        }
    }
    let env_pathbuf = env.to_path_buf();
    let remaining: Vec<PathBuf> = separate_dir_list(already_sourced_env_dirs)
        .into_iter()
        .filter(|d| d != &env_pathbuf)
        .collect();
    cmds.push(shell.set_var_not_exported(SOURCED_PROFILE_SCRIPTS_ENV, &join_dir_list(remaining)));
    cmds
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn bash_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Bash, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-bash';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-bash';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Zsh, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-zsh';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-zsh';".to_string(),
            "typeset -g _FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Tcsh, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-tcsh';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-tcsh';".to_string(),
            "set _FLOX_SOURCED_PROFILE_SCRIPTS = 'newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_all_exist_correct_order() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Fish, |_| true);
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-fish';".to_string(),
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-fish';".to_string(),
            "set -g _FLOX_SOURCED_PROFILE_SCRIPTS 'newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Bash, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-bash';".to_string(),
            "source 'newer/activate.d/profile-bash';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Zsh, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-zsh';".to_string(),
            "source 'newer/activate.d/profile-zsh';".to_string(),
            "typeset -g _FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Tcsh, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-tcsh';".to_string(),
            "source 'newer/activate.d/profile-tcsh';".to_string(),
            "set _FLOX_SOURCED_PROFILE_SCRIPTS = 'newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_some_exist() {
        let dirs = "newer:older";
        let cmds = source_profile_scripts_cmds_activate(dirs, "", &Shell::Fish, |p| {
            p != Path::new("newer/activate.d/profile-common")
        });
        let expected = vec![
            "source 'older/activate.d/profile-common';".to_string(),
            "source 'older/activate.d/profile-fish';".to_string(),
            "source 'newer/activate.d/profile-fish';".to_string(),
            "set -g _FLOX_SOURCED_PROFILE_SCRIPTS 'newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn one_already_sourced_script_is_skipped() {
        let dirs = "newer:older";
        let already_sourced = "older";
        let cmds =
            source_profile_scripts_cmds_activate(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source 'newer/activate.d/profile-common';".to_string(),
            "source 'newer/activate.d/profile-bash';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn all_already_sourced_scripts_are_skipped() {
        let dirs = "newer:older";
        let already_sourced = "newer:older";
        let cmds =
            source_profile_scripts_cmds_activate(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec!["_FLOX_SOURCED_PROFILE_SCRIPTS='newer:older';".to_string()];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn prepend_already_sourced_when_no_overlap() {
        let dirs = "standalone";
        let already_sourced = "already:existing";
        let cmds =
            source_profile_scripts_cmds_activate(dirs, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source 'standalone/activate.d/profile-common';".to_string(),
            "source 'standalone/activate.d/profile-bash';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='standalone:already:existing';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_emits_inverse_order_then_updates_tracking_var() {
        // Per-shell first, then common — inverse of activation order.
        // Tracking var starts non-empty (realistic stacked-env state) and
        // is rewritten with the deactivated env removed; the sibling stays.
        let env = Path::new("/envs/myenv");
        let already_sourced = "/envs/myenv:/envs/sibling";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-bash';".to_string(),
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS=/envs/sibling;".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn zsh_deactivate_emits_inverse_order_then_updates_tracking_var() {
        let env = Path::new("/envs/myenv");
        let already_sourced = "/envs/myenv:/envs/sibling";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Zsh, |_| true);
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-zsh';".to_string(),
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "typeset -g _FLOX_SOURCED_PROFILE_SCRIPTS=/envs/sibling;".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn fish_deactivate_emits_inverse_order_then_updates_tracking_var() {
        let env = Path::new("/envs/myenv");
        let already_sourced = "/envs/myenv:/envs/sibling";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Fish, |_| true);
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-fish';".to_string(),
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "set -g _FLOX_SOURCED_PROFILE_SCRIPTS /envs/sibling;".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn tcsh_deactivate_emits_inverse_order_then_updates_tracking_var() {
        let env = Path::new("/envs/myenv");
        let already_sourced = "/envs/myenv:/envs/sibling";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Tcsh, |_| true);
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-tcsh';".to_string(),
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "set _FLOX_SOURCED_PROFILE_SCRIPTS = /envs/sibling;".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_skips_missing_per_shell_script() {
        // Per-shell script absent; common still emitted; tracking var still updated.
        let env = Path::new("/envs/myenv");
        let cmds = source_profile_scripts_cmds_deactivate(env, "", &Shell::Bash, |p| {
            p != Path::new("/envs/myenv/activate.d/deactivate-profile-bash")
        });
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_skips_missing_common_script() {
        // Common script absent; per-shell still emitted; tracking var still updated.
        let env = Path::new("/envs/myenv");
        let cmds = source_profile_scripts_cmds_deactivate(env, "", &Shell::Bash, |p| {
            p != Path::new("/envs/myenv/activate.d/deactivate-profile-common")
        });
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-bash';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_all_missing_still_updates_tracking_var() {
        // An env with no deactivate scripts is still torn down — the
        // tracking-var update is unconditional. Mirrors the activate side,
        // which also silently no-ops when profile-* files are missing
        // (see source_profile_scripts_cmds_activate above): the env is
        // entering/leaving the stack regardless of whether it had hooks.
        let env = Path::new("/envs/myenv");
        let cmds = source_profile_scripts_cmds_deactivate(env, "", &Shell::Bash, |_| false);
        let expected = vec!["_FLOX_SOURCED_PROFILE_SCRIPTS='';".to_string()];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_removes_env_from_tracking_var() {
        // The env being deactivated is dropped from the list; siblings remain.
        let env = Path::new("/envs/inner");
        let already_sourced = "/envs/inner:/envs/outer";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Bash, |_| true);
        // No quotes around `/envs/outer`: shell_escape's whitelist
        // accepts plain paths, only escapes when special chars appear (e.g. `:`).
        let expected = vec![
            "source '/envs/inner/activate.d/deactivate-profile-bash';".to_string(),
            "source '/envs/inner/activate.d/deactivate-profile-common';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS=/envs/outer;".to_string(),
        ];
        assert_eq!(expected, cmds);
    }

    #[test]
    fn bash_deactivate_env_not_in_tracking_var_passthrough() {
        // If --env isn't in the existing list, the list comes out unchanged.
        let env = Path::new("/envs/myenv");
        let already_sourced = "/envs/a:/envs/b";
        let cmds =
            source_profile_scripts_cmds_deactivate(env, already_sourced, &Shell::Bash, |_| true);
        let expected = vec![
            "source '/envs/myenv/activate.d/deactivate-profile-bash';".to_string(),
            "source '/envs/myenv/activate.d/deactivate-profile-common';".to_string(),
            "_FLOX_SOURCED_PROFILE_SCRIPTS='/envs/a:/envs/b';".to_string(),
        ];
        assert_eq!(expected, cmds);
    }
}
