use std::path::{Path, PathBuf};

use serde::Serialize;
use thiserror::Error;

/// A path that is guaranteed to be canonicalized
///
/// [`ManagedEnvironment`] uses this to refer to the path of its `.flox` directory.
/// [`ManagedEnvironment::encode`] is used to uniquely identify the environment
/// by encoding the canonicalized path.
/// This encoding is used to create a unique branch name in the floxmeta repository.
/// Thus, rather than canonicalizing the path every time we need to encode it,
/// we store the path as a [`CanonicalPath`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, derive_more::Deref, derive_more::AsRef)]
#[deref(forward)]
#[as_ref(forward)]
pub struct CanonicalPath(PathBuf);

#[derive(Debug, Error)]
#[error("couldn't canonicalize path {path:?}: {err}")]
pub struct CanonicalizeError {
    pub path: PathBuf,
    #[source]
    pub err: std::io::Error,
}

impl CanonicalPath {
    pub fn new(path: impl AsRef<Path>) -> Result<Self, CanonicalizeError> {
        let canonicalized = std::fs::canonicalize(&path).map_err(|e| CanonicalizeError {
            path: path.as_ref().to_path_buf(),
            err: e,
        })?;
        Ok(Self(canonicalized))
    }

    pub fn into_path_buf(self) -> PathBuf {
        self.0
    }
}
