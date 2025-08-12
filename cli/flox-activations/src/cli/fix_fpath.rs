use clap::Args;

use super::fix_paths::prepend_dirs_to_pathlike_var;
use super::separate_dir_list;

#[derive(Debug, Args)]
pub struct FixFpathArgs {
    /// The contents of `$FLOX_ENV_DIRS`.
    #[arg(long)]
    pub env_dirs: String,
    /// The contents of `$FPATH` (specifically `$FPATH` and not `$fpath`).
    #[arg(long)]
    pub colon_separated_fpath: String,
}

impl FixFpathArgs {
    pub fn handle(&self) {
        let output = Self::handle_inner(&self.env_dirs, &self.colon_separated_fpath);
        println!("{output}");
    }

    fn handle_inner(env_dirs_joined: &str, colon_separated_fpath: &str) -> String {
        let env_dirs = separate_dir_list(env_dirs_joined);
        let path_dirs = separate_dir_list(colon_separated_fpath);
        let suffixes = ["share/zsh/site-functions", "share/zsh/vendor-completions"];
        let fixed_path_dirs = prepend_dirs_to_pathlike_var(&env_dirs, &suffixes, &path_dirs);
        let as_strs = fixed_path_dirs
            .into_iter()
            .map(|s| s.to_string_lossy().to_string())
            .collect::<Vec<_>>();
        format_fpath_array(&as_strs)
    }
}

fn format_fpath_array(dir_list: &[impl AsRef<str>]) -> String {
    let quoted_dirs = dir_list
        .iter()
        .map(|s| format!("\"{}\"", s.as_ref()))
        .collect::<Vec<_>>();
    let space_separated = quoted_dirs.join(" ");
    format!("fpath=({space_separated})")
}

#[cfg(test)]
mod tests {
    use super::*;

    // Most of what we would test here is already covered by tests
    // in `fix_paths.rs` since that's where `prepend_dirs_to_pathlike_var`
    // is defined.

    #[test]
    fn makes_space_separated_array() {
        let env_dirs = "foo:bar";
        let fpath = "dir1:dir2";
        let output = FixFpathArgs::handle_inner(env_dirs, fpath);
        let expected = r#"fpath=("foo/share/zsh/site-functions" "foo/share/zsh/vendor-completions" "bar/share/zsh/site-functions" "bar/share/zsh/vendor-completions" "dir1" "dir2")"#;
        assert_eq!(output.as_str(), expected);
    }
}
