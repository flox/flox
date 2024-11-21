pub mod activations;
pub mod canonical_path;
#[cfg(feature = "proc_status")]
pub mod proc_status;
mod version;

use std::io::BufWriter;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use fslock::LockFile;
use serde::Serialize;
pub use version::Version;

pub const N_HASH_CHARS: usize = 8;

/// Returns the truncated hash of a [Path]
pub fn path_hash(p: impl AsRef<Path>) -> String {
    let mut chars = blake3::hash(p.as_ref().as_os_str().as_bytes()).to_hex();
    chars.truncate(N_HASH_CHARS);
    chars.to_string()
}

#[derive(Debug, thiserror::Error)]
pub enum SerializeError {
    #[error("file stored in an invalid location: {0}")]
    InvalidLocation(PathBuf),
    #[error("failed to open temporary file")]
    OpenTmpFile(#[source] std::io::Error),
    #[error("failed to rename temporary file")]
    RenameTmpFile(#[source] tempfile::PersistError),
    #[error("failed to write temporary file")]
    WriteTmpFile(#[source] serde_json::Error),
}

/// Serialize a value and write it to disk atomically.
///
/// First the value is written to a temporary file,
/// and then it is renamed so the write appears atomic.
/// This also takes a [LockFile] argument to ensure that the write can only be
/// performed when the lock is acquired.
/// It is a bug if you pass a [LockFile] that doesn't correspond to the file, as
/// that is essentially bypassing the lock.
/// `path` must have a parent directory.
pub fn serialize_atomically<T>(
    value: &T,
    path: &impl AsRef<Path>,
    _lock: LockFile,
) -> Result<(), SerializeError>
where
    T: ?Sized + Serialize,
{
    let parent = path.as_ref().parent().ok_or(
        // This error is thrown in the unlikely scenario that `path` is:
        // - An empty string
        // - `/`
        // - `.`
        SerializeError::InvalidLocation(path.as_ref().to_path_buf()),
    )?;
    let temp_file = tempfile::NamedTempFile::new_in(parent).map_err(SerializeError::OpenTmpFile)?;

    let writer = BufWriter::new(&temp_file);
    serde_json::to_writer_pretty(writer, value).map_err(SerializeError::WriteTmpFile)?;
    temp_file
        .persist(path.as_ref())
        .map_err(SerializeError::RenameTmpFile)?;
    Ok(())
}

/// Returns a `tracing`-compatible form of a [Path]
pub fn traceable_path(p: impl AsRef<Path>) -> impl tracing::Value {
    let path = p.as_ref();
    path.display().to_string()
}
