use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use indoc::formatdoc;
use tempfile::{NamedTempFile, TempDir, TempPath, tempdir_in};

use crate::flox::{Flox, FloxhubToken};

/// Hostnames that are authenticated with FloxHub credentials.
const FLOXHUB_AUTHENTICATED_HOSTNAMES: [&str; 6] = [
    "publisher.flox.dev",
    "publisher.preview.flox.dev",
    "api.preview2.flox.dev",
    // The following should be removed after infra migrations.
    "experimental-warehouse.production2.flox.dev",
    "experimental-warehouse.preview2.flox.dev",
    "localhost",
];

pub trait AuthProvider {
    fn token(&self) -> Option<&FloxhubToken>;
    fn create_netrc(&self) -> Result<TempPath, AuthError>;
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("failed to create temporary directory")]
    CreateTempDir(#[source] std::io::Error),

    #[error("failed to create netrc")]
    CreateNetrc(#[source] std::io::Error),

    // It's intended that this error will be caught so that we can present the
    // typical friendly "you probably need to re-auth" message.
    #[error("authentication token not found")]
    NoToken,

    #[error("{0}")]
    CatchAll(String),
}

/// A method for authenticating a `nix copy`
/// TODO: this probably needs to be refactored once we have a clearer idea of
/// how auth should work.
pub enum NixCopyAuth {
    Netrc(PathBuf),
    CatalogProvided(CatalogAuth),
}

pub type CatalogAuth = serde_json::Map<String, serde_json::Value>;

pub fn catalog_auth_to_envs(auth: &CatalogAuth) -> Result<HashMap<String, String>, AuthError> {
    let Some(aws_s3) = auth.get("aws-s3") else {
        return Err(AuthError::CatchAll(
            "Only aws-s3 auth is supported".to_string(),
        ));
    };
    // Don't error if there are extra keys we don't know how to handle for
    // forwards compatibility.
    let Some(envs_value) = aws_s3.get("envs") else {
        return Err(AuthError::CatchAll(
            "Expected 'envs' object in aws-s3 auth".to_string(),
        ));
    };

    let envs = serde_json::from_value(envs_value.clone())
        .map_err(|e| AuthError::CatchAll(format!("Expected 'envs' to be a map: {e}")))?;

    Ok(envs)
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

    /// Construct a new auth provider with no token and a standalone tempdir.
    pub fn from_none() -> Result<Self, AuthError> {
        Ok(Self {
            netrc_tempdir: TempDir::new().map_err(AuthError::CreateTempDir)?,
            floxhub_token: None,
        })
    }
}

impl AuthProvider for Auth {
    /// Get a reference to the user's token (which may be expired).
    fn token(&self) -> Option<&FloxhubToken> {
        self.floxhub_token.as_ref()
    }

    /// Creates a temporary netrc file with authentication credentials
    /// and returns the path.
    fn create_netrc(&self) -> Result<TempPath, AuthError> {
        match self.floxhub_token.as_ref() {
            Some(token) => {
                write_floxhub_netrc(&self.netrc_tempdir, token).map_err(AuthError::CreateNetrc)
            },
            None => Err(AuthError::NoToken),
        }
    }
}

/// Write a `netrc` temporary file for providing FloxHub auth.
pub fn write_floxhub_netrc(
    temp_dir: impl AsRef<Path>,
    token: &FloxhubToken,
) -> std::io::Result<TempPath> {
    let token_secret = token.secret();
    // Restrict to known hostnamess so that we don't accidentally leak FloxHub
    // credentials to third-party ingress URIs.
    let netrc_contents = FLOXHUB_AUTHENTICATED_HOSTNAMES
        .iter()
        .map(|hostname| {
            // Our auth proxy only uses the "password" field from BasicAuth.
            formatdoc! {"
                machine {hostname}
                login unused
                password {token_secret}
            "}
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    let mut netrc_file = NamedTempFile::new_in(temp_dir)?;
    netrc_file.write_all(netrc_contents.as_bytes())?;
    netrc_file.flush()?;

    Ok(netrc_file.into_temp_path())
}

/// Returns true if we determine that the store URL requires an authentication
/// token. Note that this is a best guess for now and *really* means that we
/// can't tell that we *don't* need a token.
pub(crate) fn store_needs_auth(url: &str) -> bool {
    !(url.starts_with("https://cache.nixos.org") || url == "daemon")
}
