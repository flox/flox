use std::collections::{HashSet, VecDeque};
use std::io::Write;
use std::path::PathBuf;

use anyhow::bail;
use clap::Args;
use tracing::debug;

use super::{join_dir_list, separate_dir_list};

/// The environment variable holding PATH prepends registered by
/// `etc/profile.d` scripts via the `flox_prepend_path` helper.
/// The value is a colon-separated list of `<env_dir>=<prepend_dir>` pairs,
/// most recent registration first.
///
/// The name must NOT end in `PATH`: fish auto-treats variables named
/// `*PATH` as path-list variables (colon-split on import, re-joined on
/// expansion), which would corrupt the multi-entry value in transit.
pub const FLOX_ENV_PATH_PREPENDS_VAR: &str = "_FLOX_ENV_PATH_PREPENDS";

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
    #[arg(help = "The contents of _FLOX_ENV_PATH_PREPENDS.")]
    #[arg(long, value_name = "STRING", default_value = "")]
    pub path_prepends: String,
}

impl FixPathsArgs {
    pub fn handle_inner(&self, output: &mut impl Write) -> Result<(), anyhow::Error> {
        debug!(
            "Preparing to fix path vars, FLOX_ENV_DIRS={}",
            self.env_dirs
        );
        let new_path = fix_path_var(&self.env_dirs, &self.path, &self.path_prepends);
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

/// Splits the contents of _FLOX_ENV_PATH_PREPENDS into
/// (environment dir, prepend dir) pairs.
/// Entries without a '=' or with an empty half are ignored.
///
/// Entries are never pruned from the variable itself: registrations for
/// deactivated environments accumulate there and are filtered out at
/// replay time by `prepend_dirs_to_pathlike_var`, which drops entries
/// whose environment is not in FLOX_ENV_DIRS. Deactivation restores the
/// variable to its pre-activation value via the recorded env diff, so
/// accumulation is bounded by the depth of the activation stack.
fn separate_prepends_list(joined: &str) -> Vec<(PathBuf, PathBuf)> {
    let joined = if joined == "empty" { "" } else { joined };
    joined
        .split(':')
        .filter_map(|entry| {
            let (env_dir, prepend_dir) = entry.split_once('=')?;
            if env_dir.is_empty() || prepend_dir.is_empty() {
                return None;
            }
            Some((PathBuf::from(env_dir), PathBuf::from(prepend_dir)))
        })
        .collect()
}

/// Adds subdirectories of FLOX_ENV_DIRS to the front of the provided list
/// of directories.
/// If suffixes is empty, adds each dir in FLOX_ENV_DIRS directly.
/// Suffixes are expected *not* to contain a leading slash.
/// Each entry of `per_env_prepends` is placed ahead of the suffix dirs of
/// the environment it is registered against, retaining that environment's
/// position in the layered ordering. Entries registered against an
/// environment not present in `flox_env_dirs` are dropped.
pub fn prepend_dirs_to_pathlike_var(
    flox_env_dirs: &[PathBuf],
    suffixes: &[impl AsRef<str>],
    existing_dirs: &[PathBuf],
    per_env_prepends: &[(PathBuf, PathBuf)],
) -> Vec<PathBuf> {
    let mut dir_set = HashSet::new();
    let mut dirs = VecDeque::new();
    // Directories at the front of the list have been activated most recently,
    // so their directories should go at the front of the list. However, if
    // we just iterate in the typical order and prepend those directories to
    // PATH, you'll get those directories in reverse order of activation, so
    // we iterate in reverse order while prepending.
    for dir in flox_env_dirs.iter().rev() {
        if suffixes.is_empty() {
            // If no suffixes, add the directory directly
            if dir_set.insert(dir.clone()) {
                dirs.push_front(dir.clone())
            }
        } else {
            for suffix in suffixes.iter().rev() {
                let new_dir = dir.join(suffix.as_ref());
                // Insert returns `true` if the value was _newly_ inserted
                if dir_set.insert(new_dir.clone()) {
                    dirs.push_front(new_dir)
                }
            }
        }
        // Registered prepends go ahead of this environment's own dirs,
        // mirroring what a plain `PATH="$dir:$PATH"` expressed at
        // registration time. The list is most-recent-first, so reverse
        // iteration with push_front keeps the most recent registration
        // foremost within the layer.
        for (env_dir, prepend_dir) in per_env_prepends.iter().rev() {
            if env_dir == dir && dir_set.insert(prepend_dir.clone()) {
                dirs.push_front(prepend_dir.clone())
            }
        }
    }
    // Deactivated environments' registrations are dropped as a matter of
    // course, but an entry that never matches is otherwise silent and hard
    // to debug — e.g. an environment path containing '=' mangled by the
    // unescaped "<env>=<dir>" encoding.
    for (env_dir, prepend_dir) in per_env_prepends {
        if !flox_env_dirs.contains(env_dir) {
            debug!(
                env_dir = %env_dir.display(),
                prepend_dir = %prepend_dir.display(),
                "dropping PATH prepend registered against an environment not in FLOX_ENV_DIRS"
            );
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

/// Calculate the new PATH variable from FLOX_ENV_DIRS, the existing PATH,
/// and any prepends registered in _FLOX_ENV_PATH_PREPENDS.
pub fn fix_path_var(flox_env_dirs_var: &str, path_var: &str, path_prepends_var: &str) -> String {
    let path_dirs = separate_dir_list(path_var);
    let flox_env_dirs = separate_dir_list(flox_env_dirs_var);
    let per_env_prepends = separate_prepends_list(path_prepends_var);
    let suffixes = ["bin", "sbin"];
    let fixed_path_dirs =
        prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs, &per_env_prepends);
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
pub fn fix_manpath_var(flox_env_dirs_var: &str, manpath_var: &str) -> String {
    let has_leading_colon = manpath_var.starts_with(':');
    let has_double_colon = manpath_var.contains("::");
    let has_trailing_colon = manpath_var.ends_with(':');
    let manpath_dirs = separate_dir_list(manpath_var);
    let flox_env_dirs = separate_dir_list(flox_env_dirs_var);
    let suffixes = ["share/man"];
    let fixed_manpath_dirs =
        prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &manpath_dirs, &[]);
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
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs, &[]);
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
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs, &[]);
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
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs, &[]);
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
                path_prepends: String::new(),
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
        let new_dirs = prepend_dirs_to_pathlike_var(&flox_env_dirs, &suffixes, &path_dirs, &[]);
        assert_eq!(new_dirs, vec![
            PathBuf::from_str("/flox_env1/bin").unwrap(),
            PathBuf::from_str("/flox_env1/sbin").unwrap(),
            PathBuf::from_str("/path1").unwrap(),
        ]);
    }

    #[test]
    fn handles_blank_manpath() {
        let env_dirs = "/env1:/env2";
        let manpath = "";
        let fixed = fix_manpath_var(env_dirs, manpath);
        assert_eq!(fixed, "/env1/share/man:/env2/share/man:");
    }

    #[test]
    fn handles_empty_manpath() {
        let env_dirs = "/env1:/env2";
        let manpath = "empty";
        let fixed = fix_manpath_var(env_dirs, manpath);
        assert_eq!(fixed, "/env1/share/man:/env2/share/man:");
    }

    #[test]
    fn fixing_paths_is_idempotent() {
        let env_dirs = "/env1:/env2";
        let path = "/foo:/bar";
        let manpath = "/baz:/qux";
        let fixed_path = fix_path_var(env_dirs, path, "");
        let fixed_manpath = fix_manpath_var(env_dirs, manpath);
        let fixed_path_again = fix_path_var(env_dirs, &fixed_path, "");
        let fixed_manpath_again = fix_manpath_var(env_dirs, &fixed_manpath);
        assert_eq!(fixed_path, fixed_path_again);
        assert_eq!(fixed_manpath, fixed_manpath_again);
    }

    #[test]
    fn prepends_are_placed_at_their_environments_layer() {
        let env_dirs = "/env1:/env2";
        let path = "/env1/prepend:/foo:/bar";
        let prepends = "/env2=/env2/prepend:/env1=/env1/prepend";
        let fixed = fix_path_var(env_dirs, path, prepends);
        assert_eq!(
            fixed,
            "/env1/prepend:/env1/bin:/env1/sbin:/env2/prepend:/env2/bin:/env2/sbin:/foo:/bar"
        );
    }

    #[test]
    fn most_recent_prepend_is_foremost_within_a_layer() {
        let env_dirs = "/env1";
        let path = "/foo";
        // Most recent registration first, as built by `flox_prepend_path`.
        let prepends = "/env1=/newer:/env1=/older";
        let fixed = fix_path_var(env_dirs, path, prepends);
        assert_eq!(fixed, "/newer:/older:/env1/bin:/env1/sbin:/foo");
    }

    #[test]
    fn prepends_for_inactive_environments_are_dropped() {
        let env_dirs = "/env1";
        let path = "/env2/prepend:/foo";
        let prepends = "/env2=/env2/prepend";
        let fixed = fix_path_var(env_dirs, path, prepends);
        // The dir is no longer treated as a prepend, but an existing PATH
        // entry is preserved in place like any other inherited dir.
        assert_eq!(fixed, "/env1/bin:/env1/sbin:/env2/prepend:/foo");
    }

    #[test]
    fn malformed_and_sentinel_prepend_entries_are_ignored() {
        assert_eq!(separate_prepends_list("empty"), vec![]);
        assert_eq!(separate_prepends_list(""), vec![]);
        assert_eq!(separate_prepends_list("no-equals-sign"), vec![]);
        assert_eq!(separate_prepends_list("=/dir:/env=:"), vec![]);
        assert_eq!(separate_prepends_list("/env=/dir"), vec![(
            PathBuf::from("/env"),
            PathBuf::from("/dir")
        )]);
    }

    #[test]
    fn fixing_paths_with_prepends_is_idempotent() {
        let env_dirs = "/env1:/env2";
        let path = "/foo:/bar";
        let prepends = "/env1=/env1/prepend:/env2=/env2/prepend";
        let fixed = fix_path_var(env_dirs, path, prepends);
        let fixed_again = fix_path_var(env_dirs, &fixed, prepends);
        assert_eq!(fixed, fixed_again);
    }

    #[test]
    fn same_dir_registered_by_two_envs_keeps_innermost_position() {
        let env_dirs = "/env1:/env2";
        let path = "/foo";
        let prepends = "/env2=/shared:/env1=/shared";
        let fixed = fix_path_var(env_dirs, path, prepends);
        // A dir registered by multiple active environments is deduped to
        // the outermost registering environment's layer: it keeps the
        // position it already held when the inner environment layered on
        // top, exactly as an inherited PATH entry would.
        assert_eq!(
            fixed,
            "/env1/bin:/env1/sbin:/shared:/env2/bin:/env2/sbin:/foo"
        );
    }

    #[test]
    fn prepend_equal_to_own_bin_dir_is_deduped() {
        let env_dirs = "/env1";
        let path = "/foo";
        let prepends = "/env1=/env1/bin";
        let fixed = fix_path_var(env_dirs, path, prepends);
        // bin/sbin are pushed before prepends are considered, so the
        // registration dedups away rather than duplicating the entry.
        assert_eq!(fixed, "/env1/bin:/env1/sbin:/foo");
    }

    #[test]
    fn prepend_dirs_containing_spaces_are_preserved() {
        let env_dirs = "/env one";
        let path = "/foo";
        let prepends = "/env one=/env one/extra bin";
        let fixed = fix_path_var(env_dirs, path, prepends);
        assert_eq!(fixed, "/env one/extra bin:/env one/bin:/env one/sbin:/foo");
    }
}
