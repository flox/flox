use std::io;
use std::path::PathBuf;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum IoError {
    #[error("Couldn't create temp dir in {dir}: {err}")]
    CreateTempDir { dir: PathBuf, err: io::Error },
    #[error("Couldn't open {file}: {err}")]
    Open { file: PathBuf, err: io::Error },
    #[error("Couldn't copy {file}: {err}")]
    Copy { file: PathBuf, err: io::Error },
    #[error("Couldn't write {file}: {err}")]
    Write { file: PathBuf, err: io::Error },
    #[error("Path {dir} does not exist or is invalid: {err}")]
    Canonicalize { dir: PathBuf, err: io::Error },
    #[error("Couldn't create directory {dir}: {err}")]
    CreateDir { dir: PathBuf, err: io::Error },
    #[error("Couldn't make file '{file}' read-only: {err}")]
    MakeReadonly { file: PathBuf, err: io::Error },
    #[error("Couldn't make file '{file}' writable: {err}")]
    MakeWritable { file: PathBuf, err: io::Error },
    #[error("Couldn't get metadata for '{file}': {err}")]
    GetMetadata { file: PathBuf, err: io::Error },
}
