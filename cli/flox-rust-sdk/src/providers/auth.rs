use std::path::Path;

use tempfile::{TempDir, tempdir_in};

use crate::flox::{Flox, FloxhubToken};

pub trait AuthProvider {
    fn token(&self) -> Option<&FloxhubToken>;
    fn tempdir_path(&self) -> &Path;
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("failed to create temporary directory")]
    CreateTempDir(#[source] std::io::Error),

    #[error("{0}")]
    CatchAll(String),
}

/// Handles authentication with catalog stores during build and publish.
#[derive(Debug)]
pub struct Auth {
    /// The directory in which we'll create an ad-hoc netrc file if needed.
    netrc_tempdir: TempDir,
    /// The user's FloxHub authentication token.
    floxhub_token: Option<FloxhubToken>,
}

impl Auth {
    /// Construct a new auth provider from a Flox instance
    pub fn from_flox(flox: &Flox) -> Result<Self, AuthError> {
        Ok(Self {
            floxhub_token: flox.floxhub_token.clone(),
            netrc_tempdir: tempdir_in(&flox.temp_dir).map_err(AuthError::CreateTempDir)?,
        })
    }

    /// Construct a new auth provider from a tempdir and a token.
    pub fn from_tempdir_and_token(tempdir: TempDir, token: Option<FloxhubToken>) -> Self {
        Self {
            netrc_tempdir: tempdir,
            floxhub_token: token,
        }
    }
}

impl AuthProvider for Auth {
    /// Get a reference to the user's token (which may be expired).
    fn token(&self) -> Option<&FloxhubToken> {
        self.floxhub_token.as_ref()
    }

    /// Get the location of the tempdir in which an ad-hoc netrc
    /// can be created.
    fn tempdir_path(&self) -> &Path {
        self.netrc_tempdir.path()
    }
}
