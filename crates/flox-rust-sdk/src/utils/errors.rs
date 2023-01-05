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
}
