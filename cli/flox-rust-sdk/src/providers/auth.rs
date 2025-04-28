use std::path::Path;

use tempfile::TempDir;

use crate::flox::FloxhubToken;

/// Handles authentication with catalog stores during build and publish.
#[derive(Debug)]
pub struct Auth {
    /// The directory in which we'll create an ad-hoc netrc file if needed.
    netrc_tempdir: TempDir,
    /// The user's FloxHub authentication token.
    floxhub_token: Option<FloxhubToken>,
}

impl Auth {
    /// Get a reference to the user's token (which may be expired).
    pub fn token(&self) -> Option<&FloxhubToken> {
        self.floxhub_token.as_ref()
    }

    /// Get the location of the tempdir in which an ad-hoc netrc
    /// can be created.
    pub fn tempdir_path(&self) -> &Path {
        self.netrc_tempdir.path()
    }
}
