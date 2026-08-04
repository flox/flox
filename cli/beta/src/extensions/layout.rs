//! On-disk layout under `$XDG_DATA_HOME/flox/extensions/`.
//!
//! The managed directory is rooted at `flox.data_dir.join("extensions")`.
//! Each installed extension lives in its own `flox-<name>/` subdirectory
//! containing the `flox-<name>` executable, the optional copied
//! `flox-extension.toml` author manifest, and a `state.toml` describing
//! the install. A single `.lock` file at the root serializes mutating
//! operations (install / remove / upgrade); `list` and the dispatch
//! `find` are lock-free.

use std::path::PathBuf;

use flox_rust_sdk::flox::Flox;

pub fn extensions_root(flox: &Flox) -> PathBuf {
    flox.data_dir.join("extensions")
}

pub fn install_dir(flox: &Flox, name: &str) -> PathBuf {
    extensions_root(flox).join(format!("flox-{name}"))
}

pub fn state_path(flox: &Flox, name: &str) -> PathBuf {
    install_dir(flox, name).join("state.toml")
}

pub fn lock_path(flox: &Flox) -> PathBuf {
    extensions_root(flox).join(".lock")
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn extensions_root_is_under_data_dir() {
        let (flox, _tempdir) = flox_instance();
        assert_eq!(extensions_root(&flox), flox.data_dir.join("extensions"));
    }

    #[test]
    fn install_dir_prefixes_flox() {
        let (flox, _tempdir) = flox_instance();
        assert_eq!(
            install_dir(&flox, "hello"),
            flox.data_dir.join("extensions").join("flox-hello"),
        );
    }

    #[test]
    fn state_and_lock_paths() {
        let (flox, _tempdir) = flox_instance();
        let root = flox.data_dir.join("extensions");
        assert_eq!(
            state_path(&flox, "hello"),
            root.join("flox-hello").join("state.toml")
        );
        assert_eq!(lock_path(&flox), root.join(".lock"));
    }
}
