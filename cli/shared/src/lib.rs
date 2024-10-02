pub mod canonical_path;
mod version;

use std::os::unix::ffi::OsStrExt;
use std::path::Path;

pub use version::Version;

pub const N_HASH_CHARS: usize = 8;

/// Returns the truncated hash of a [Path]
pub fn path_hash(p: impl AsRef<Path>) -> String {
    let mut chars = blake3::hash(p.as_ref().as_os_str().as_bytes()).to_hex();
    chars.truncate(N_HASH_CHARS);
    chars.to_string()
}
