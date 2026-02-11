pub mod activate;
pub mod activations;
pub mod canonical_path;
#[cfg(feature = "proc_status")]
pub mod proc_status;
pub mod process_compose;
pub mod sentry;
pub mod util;
pub mod vars;
mod version;

use std::fmt::Display;
use std::io::{BufWriter, Write};
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
pub enum WriteError {
    /// This error is thrown in the unlikely scenario that the path being
    /// written to is:
    /// - An empty string
    /// - `/`
    /// - `.`
    #[error("file stored in an invalid location: {0}")]
    InvalidLocation(PathBuf),
    #[error("failed to open temporary file")]
    OpenTmpFile(#[source] std::io::Error),
    #[error("failed to rename temporary file")]
    RenameTmpFile(#[source] tempfile::PersistError),
    #[error("failed to write temporary file")]
    SerdeWriteTmpFile(#[source] serde_json::Error),
    #[error("failed to write temporary file")]
    WriteTmpFile(#[source] std::io::Error),
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
) -> Result<(), WriteError>
where
    T: ?Sized + Serialize,
{
    let parent = path
        .as_ref()
        .parent()
        .ok_or(WriteError::InvalidLocation(path.as_ref().to_path_buf()))?;
    let temp_file = tempfile::NamedTempFile::new_in(parent).map_err(WriteError::OpenTmpFile)?;

    let writer = BufWriter::new(&temp_file);
    serde_json::to_writer_pretty(writer, value).map_err(WriteError::SerdeWriteTmpFile)?;
    temp_file
        .persist(path.as_ref())
        .map_err(WriteError::RenameTmpFile)?;
    Ok(())
}

// At the moment this could be in flox-rust-sdk but I think it should be
// co-located with serialize_atomically
/// Write contents to a file atomically by renaming a tempfile
pub fn write_atomically(
    path: &impl AsRef<Path>,
    contents: impl AsRef<[u8]>,
) -> Result<(), WriteError> {
    // Create the tempfile in the same directory as the file so persist()
    // doesn't run into a cross device linking error
    let parent = path
        .as_ref()
        .parent()
        .ok_or(WriteError::InvalidLocation(path.as_ref().to_path_buf()))?;

    let mut tempfile = tempfile::NamedTempFile::new_in(parent).map_err(WriteError::OpenTmpFile)?;

    tempfile
        .write_all(contents.as_ref())
        .map_err(WriteError::WriteTmpFile)?;

    tempfile
        .persist(path.as_ref())
        .map_err(WriteError::RenameTmpFile)?;
    Ok(())
}

/// Returns a `tracing`-compatible form of a [Path]
pub fn traceable_path(p: impl AsRef<Path>) -> impl tracing::Value {
    let path = p.as_ref();
    path.display().to_string()
}

/// Returns a `tracing`-compatible form of an `Option<PathBuf>`
pub fn maybe_traceable_path(maybe_path: &Option<PathBuf>) -> impl tracing::Value {
    if let Some(p) = maybe_path {
        p.display().to_string()
    } else {
        String::from("null")
    }
}

/// Returns a log file name, or glob pattern, for upgrade-check logs.
pub fn log_file_format_upgrade_check(index: impl Display) -> String {
    format!("upgrade-check.{}.log", index)
}
