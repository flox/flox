use clap::Args;

use super::fix_paths::prepend_dirs_to_pathlike_var;
use super::{join_dir_list, separate_dir_list};

#[derive(Debug, Args)]
pub struct PrependAndDedupArgs {
    /// The contents of `$FLOX_ENV_DIRS`.
    #[arg(long)]
    pub env_dirs: String,
    /// The contents of a `PATH`-like variable e.g. a colon-delimited
    /// list of directories.
    #[arg(long)]
    pub path_like: String,
    /// The suffix to append to each environment directory.
    #[arg(long)]
    pub suffix: String,
}

impl PrependAndDedupArgs {
    pub fn handle(&self) {
        let output = Self::handle_inner(&self.env_dirs, &self.suffix, &self.path_like);
        println!("{output}");
    }

    fn handle_inner(env_dirs_joined: &str, suffix: &str, path_like: &str) -> String {
        let env_dirs = separate_dir_list(env_dirs_joined);
        let path_dirs = separate_dir_list(path_like);
        let fixed_path_dirs = prepend_dirs_to_pathlike_var(&env_dirs, &[suffix], &path_dirs);
        join_dir_list(fixed_path_dirs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Most of what we would test here is already covered by tests
    // in `fix_paths.rs` since that's where `prepend_dirs_to_pathlike_var`
    // is defined.

    #[test]
    fn handles_empty_pathlike_var() {
        let env_dirs = "foo:bar";
        let suffix = "bin";
        let output = PrependAndDedupArgs::handle_inner(env_dirs, suffix, "");
        assert_eq!(output, "foo/bin:bar/bin");
    }
}
