use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;

use anyhow::bail;
use clap::Args;
use log::debug;

use super::{join_dir_list, separate_dir_list};

#[derive(Debug, Args)]
pub struct FixPathsArgs {
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

impl FixPathsArgs {
    pub fn handle_inner(&self, output: &mut impl Write) -> Result<(), anyhow::Error> {
        debug!(
            "Preparing to fix path vars, FLOX_ENV_DIRS={}",
            self.env_dirs
        );
        let new_path = fix_path_var(&self.env_dirs, &self.path);
        let new_manpath = fix_manpath_var(&self.env_dirs, &self.manpath);
        let sourceable_commands = match self.shell.as_ref() {
            "bash" => {
                let path_export = format!("export PATH=\"{new_path}\"");
                let manpath_export = format!("export MANPATH=\"{new_manpath}\"");
                format!("{path_export};\n{manpath_export};\n")
            },
            "zsh" => {
                let path_export = format!("export PATH=\"{new_path}\"");
                let manpath_export = format!("export MANPATH=\"{new_manpath}\"");
                format!("{path_export};\n{manpath_export};\n")
            },
            "fish" => {
                let path_export = format!("set -gx PATH \"{new_path}\"");
                let manpath_export = format!("set -gx MANPATH \"{new_manpath}\"");
                format!("{path_export};\n{manpath_export};\n")
            },
            "tcsh" => {
                let path_export = format!("setenv PATH \"{new_path}\"");
                let manpath_export = format!("setenv MANPATH \"{new_manpath}\"");
                format!("{path_export};\n{manpath_export};\n")
            },
            other => {
                bail!("invalid shell: {other}")
            },
        };
        output.write_all(sourceable_commands.as_bytes())?;
        Ok(())
    }

    pub fn handle(&self) -> Result<(), anyhow::Error> {
        let mut stdout = std::io::stdout();
        self.handle_inner(&mut stdout)?;
        Ok(())
    }
}

/// Adds subdirectories of FLOX_ENV_DIRS to the front of the provided list
/// of directories.
fn prepend_dirs_to_pathlike_var(
    flox_env_dirs: &[PathBuf],
    suffixes: &[&str],
    existing_dirs: &[PathBuf],
) -> Vec<PathBuf> {
    let mut dir_set = HashSet::new();
    let mut dirs = VecDeque::new();
    // Directories at the front of the list have been activated most recently,
    // so their directories should go at the front of the list. However, if
    // we just iterate in the typical order and prepend those directories to
    // PATH, you'll get those directories in reverse order of activation, so
    // we iterate in reverse order while prepending.
    for dir in flox_env_dirs.iter().rev() {
        for suffix in suffixes.iter().rev() {
            let new_dir = dir.join(suffix);
            // Insert returns `true` if the value was _newly_ inserted
            if dir_set.insert(new_dir.clone()) {
                dirs.push_front(new_dir)
            }
        }
    }
    // This is an empty string caused by splitting on a leading ':' or by
    // splitting a '::' (as found in MANPATH). We don't want to dedup these.
    let empty = PathBuf::from("");

    // By populating the set with our Flox dirs first, we ensure that they'll
    // always be at the front of the dir list and subsequent copies will be
    // deduped.
    for existing_dir in existing_dirs.iter() {
        if existing_dir == &empty {
            dirs.push_back(empty.clone());
        }
        // Insert returns `true` if the value was _newly_ inserted
        else if dir_set.insert(existing_dir.clone()) {
            dirs.push_back(existing_dir.clone());
        }
    }

    dirs.into_iter().collect::<Vec<_>>()
}

/// Calculate the new PATH variable from FLOX_ENV_DIRS and the existing PATH.
fn fix_path_var(flox_env_dirs_var: &str, path_var: &str) -> String {
    let path_dirs = separate_dir_list(path_var);
    let flox_env_dirs = separate_dir_list(flox_env_dirs_var);
    let suffixes = ["bin", "sbin"];
    let fixed_path_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs);
    join_dir_list(fixed_path_dirs)
}

/// Calculate the new man(1) search path from FLOX_ENV_DIRS and the existing MANPATH.
///
/// Note that we *always* add a trailing ':' to MANPATH because the search path
/// for man pages is complicated.
///
/// Depending on whether leading/trailing ':' characters are present in MANPATH,
/// you get different behavior. Let "STD" be the standard search path.
/// ex.) (leading) MANPATH=":foo" -> search path is "STD:foo"
/// ex.) (trailing) MANPATH="foo:" -> search path is "foo:STD"
/// ex.) (two consecutive) MANPATH="foo::bar" -> search path is "foo:STD:bar"
/// ex.) (not leading or trailing) MANPATH="foo" -> search path is "foo"
///
/// So, we always put a trailing ':' in MANPATH so that man pages from the
/// active environments take precedence without *removing* the standard
/// search path.
fn fix_manpath_var(flox_env_dirs_var: &str, manpath_var: &str) -> String {
    let has_leading_colon = manpath_var.starts_with(':');
    let has_double_colon = manpath_var.contains("::");
    let has_trailing_colon = manpath_var.ends_with(':');
    let manpath_dirs = separate_dir_list(manpath_var);
    let flox_env_dirs = separate_dir_list(flox_env_dirs_var);
    let suffixes = ["share/man"];
    let fixed_manpath_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &manpath_dirs);
    let mut joined = join_dir_list(fixed_manpath_dirs);
    if !(has_leading_colon || has_trailing_colon || has_double_colon) {
        joined.push(':');
    }
    joined
}

#[cfg(test)]
mod test {
    use std::io::BufRead;
    use std::str::FromStr;

    use super::*;

    #[test]
    fn appends_suffixes() {
        let suffixes = ["suffix"];
        let flox_env_dirs = [PathBuf::from_str("/flox_env").unwrap()];
        let path_dirs = [
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
        ];
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs);
        assert_eq!(new_dirs[0], PathBuf::from_str("/flox_env/suffix").unwrap());
    }

    #[test]
    fn flox_envs_added_in_order() {
        let suffixes = ["suffix"];
        let flox_env_dirs = [
            PathBuf::from_str("/flox_env1").unwrap(),
            PathBuf::from_str("/flox_env2").unwrap(),
        ];
        let path_dirs = [
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
        ];
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs);
        assert_eq!(new_dirs, vec![
            PathBuf::from_str("/flox_env1/suffix").unwrap(),
            PathBuf::from_str("/flox_env2/suffix").unwrap(),
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
        ]);
    }

    #[test]
    fn duplicate_paths_removed() {
        let suffixes = ["suffix"];
        let flox_env_dirs = [
            PathBuf::from_str("/flox_env1").unwrap(),
            PathBuf::from_str("/flox_env1").unwrap(),
            PathBuf::from_str("/flox_env2").unwrap(),
            PathBuf::from_str("/flox_env2").unwrap(),
        ];
        let path_dirs = [
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
        ];
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs);
        assert_eq!(new_dirs, vec![
            PathBuf::from_str("/flox_env1/suffix").unwrap(),
            PathBuf::from_str("/flox_env2/suffix").unwrap(),
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/path2").unwrap(),
        ]);
    }

    #[test]
    fn manpath_without_trailing_colon_gets_trailing_colon() {
        let env_dirs = "/foo:/bar";
        let manpath = "/baz:/qux";
        let new_manpath = fix_manpath_var(env_dirs, manpath);
        assert_eq!("/foo/share/man:/bar/share/man:/baz:/qux:", new_manpath);
    }

    #[test]
    fn manpath_with_trailing_colon_doesnt_get_new_trailing_colon() {
        let env_dirs = "/foo:/bar";
        let manpath = "/baz:/qux:";
        let new_manpath = fix_manpath_var(env_dirs, manpath);
        assert_eq!("/foo/share/man:/bar/share/man:/baz:/qux:", new_manpath);
    }

    #[test]
    fn manpath_double_colon_is_preserved() {
        // Note: this checks that the double colon is preserved _and_ that a
        //       trailing colon isn't added if the double colon exists
        let env_dirs = "/foo:/bar";
        let manpath = "/baz::/qux";
        let new_manpath = fix_manpath_var(env_dirs, manpath);
        assert_eq!("/foo/share/man:/bar/share/man:/baz::/qux", new_manpath);
    }

    #[test]
    fn manpath_with_leading_colon_gets_two_colons() {
        let env_dirs = "/foo:/bar";
        let manpath = ":/baz:/qux";
        let new_manpath = fix_manpath_var(env_dirs, manpath);
        assert_eq!("/foo/share/man:/bar/share/man::/baz:/qux", new_manpath);
    }

    #[test]
    fn lines_have_trailing_semicolons() {
        let shells = ["bash", "zsh", "fish", "tcsh"];
        let env_dirs = "/env1:/env2";
        let path = "/foo:/bar";
        let manpath = "/baz:/qux";
        for shell in shells.iter() {
            let mut buf = vec![];
            FixPathsArgs {
                shell: shell.to_string(),
                env_dirs: env_dirs.to_string(),
                path: path.to_string(),
                manpath: manpath.to_string(),
            }
            .handle_inner(&mut buf)
            .unwrap();
            for line in buf.lines() {
                assert!(line.unwrap().ends_with(';'));
            }
        }
    }

    #[test]
    fn flox_envs_moved_to_front() {
        let suffixes = ["bin", "sbin"];
        let flox_env_dirs = [PathBuf::from_str("/flox_env1").unwrap()];
        let path_dirs = [
            PathBuf::from_str("/path1").unwrap(),
            PathBuf::from_str("/flox_env1/bin").unwrap(),
        ];
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs);
        assert_eq!(new_dirs, vec![
            PathBuf::from_str("/flox_env1/bin").unwrap(),
            PathBuf::from_str("/flox_env1/sbin").unwrap(),
            PathBuf::from_str("/path1").unwrap(),
        ]);
    }

    #[test]
    fn handles_empty_manpath() {
        let env_dirs = "/env1:/env2";
        let manpath = "";
        let fixed = fix_manpath_var(env_dirs, manpath);
        assert_eq!(fixed, "/env1/share/man:/env2/share/man:");
    }

    #[test]
    fn fixing_paths_is_idempotent() {
        let env_dirs = "/env1:/env2";
        let path = "/foo:/bar";
        let manpath = "/baz:/qux";
        let fixed_path = fix_path_var(env_dirs, path);
        let fixed_manpath = fix_manpath_var(env_dirs, manpath);
        let fixed_path_again = fix_path_var(env_dirs, &fixed_path);
        let fixed_manpath_again = fix_manpath_var(env_dirs, &fixed_manpath);
        assert_eq!(fixed_path, fixed_path_again);
        assert_eq!(fixed_manpath, fixed_manpath_again);
    }
}
