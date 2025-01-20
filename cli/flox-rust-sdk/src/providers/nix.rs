pub mod test_helpers {
    use std::path::PathBuf;

    use crate::providers::buildenv::NIX_BIN;

    /// Returns a Nix store path that's known to exist.
    pub fn known_store_path() -> PathBuf {
        NIX_BIN.to_path_buf()
    }
}
