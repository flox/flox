use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use floxhub_client::AuthContext;
use indoc::formatdoc;
use tempfile::{NamedTempFile, TempDir, TempPath, tempdir_in};

use crate::flox::Flox;

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
    /// Whether a token is present to authenticate with.
    fn has_credential(&self) -> bool;
    fn create_netrc(&self) -> Result<TempPath, AuthError>;
    /// Attempt to create a netrc file, returning it if the user has a valid
    /// token, or `None` when they don't.
    ///
    /// The caller must hold the returned `TempPath` alive for as long as the
    /// file is needed; dropping it deletes the underlying file.
    fn try_create_netrc(&self) -> Option<TempPath>;
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

    // If we pass through additional secrets, we should add them to SENSITIVE_ENV_VARS
    let envs = serde_json::from_value(envs_value.clone())
        .map_err(|e| AuthError::CatchAll(format!("Expected 'envs' to be a map: {e}")))?;

    Ok(envs)
}

/// Handles authentication with catalog stores during build and publish.
#[derive(Debug)]
pub struct NixAuth {
    /// The directory in which we'll create an ad-hoc netrc file if needed.
    /// `None` for auth modes that don't use netrc (e.g. Kerberos).
    netrc_tempdir: Option<TempDir>,
    /// The user's credential; its token secret (if any) goes in the netrc.
    auth_context: AuthContext,
}

impl NixAuth {
    /// Construct a new auth provider from a Flox instance
    pub fn from_flox(flox: &Flox) -> Result<Self, AuthError> {
        let netrc_tempdir = match &flox.auth_context {
            AuthContext::Auth0(_) | AuthContext::Pat(_) => {
                Some(tempdir_in(&flox.temp_dir).map_err(AuthError::CreateTempDir)?)
            },
            AuthContext::Kerberos(_) => None,
        };
        Ok(Self {
            netrc_tempdir,
            auth_context: flox.auth_context.clone(),
        })
    }

    /// Construct a new auth provider from a tempdir and a credential.
    pub fn from_tempdir_and_context(tempdir: TempDir, auth_context: AuthContext) -> Self {
        Self {
            netrc_tempdir: Some(tempdir),
            auth_context,
        }
    }
}

impl AuthProvider for NixAuth {
    fn has_credential(&self) -> bool {
        self.auth_context.token_secret().is_some()
    }

    /// Creates a temporary netrc file with authentication credentials
    /// and returns the path.
    fn create_netrc(&self) -> Result<TempPath, AuthError> {
        let tempdir = self.netrc_tempdir.as_ref().ok_or(AuthError::NoToken)?;
        write_floxhub_netrc(tempdir, &self.auth_context)
    }

    fn try_create_netrc(&self) -> Option<TempPath> {
        let tempdir = self.netrc_tempdir.as_ref()?;
        write_floxhub_netrc(tempdir, &self.auth_context).ok()
    }
}

/// Write a `netrc` temporary file for providing FloxHub auth.
///
/// This is the one place the raw token secret is extracted from the
/// credential — it has to be written into the netrc contents.
pub fn write_floxhub_netrc(
    temp_dir: impl AsRef<Path>,
    auth_context: &AuthContext,
) -> Result<TempPath, AuthError> {
    let token_secret = auth_context.token_secret().ok_or(AuthError::NoToken)?;
    // Restrict to known hostnames so that we don't accidentally leak FloxHub
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

    let mut netrc_file = NamedTempFile::new_in(temp_dir).map_err(AuthError::CreateNetrc)?;
    netrc_file
        .write_all(netrc_contents.as_bytes())
        .map_err(AuthError::CreateNetrc)?;
    netrc_file.flush().map_err(AuthError::CreateNetrc)?;

    Ok(netrc_file.into_temp_path())
}

/// Returns true if we determine that the store URL requires an authentication
/// token. Note that this is a best guess for now and *really* means that we
/// can't tell that we *don't* need a token.
pub(crate) fn store_needs_auth(url: &str) -> bool {
    !(url.starts_with("https://cache.nixos.org") || url == "daemon")
}

#[cfg(test)]
mod tests {
    use floxhub_client::token_test_helpers::FAKE_TOKEN;
    use tempfile::tempdir;

    use super::*;
    use crate::flox::FloxhubToken;

    fn test_auth() -> NixAuth {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string()).unwrap();
        NixAuth::from_tempdir_and_context(tempdir().unwrap(), AuthContext::Auth0(Some(token)))
    }

    /// create_netrc returns a TempPath whose underlying file exists while held
    /// and is deleted when dropped — verifying the fix for the bug where the
    /// file was dropped inside a .map() closure before nix could read it.
    #[test]
    fn create_netrc_file_lives_until_dropped() {
        let auth = test_auth();
        let temp_path = auth.create_netrc().expect("create_netrc should succeed");
        let path = temp_path.to_path_buf();
        assert!(
            path.exists(),
            "netrc file should exist while TempPath is held"
        );
        drop(temp_path);
        assert!(
            !path.exists(),
            "netrc file should be deleted when TempPath is dropped"
        );
    }

    /// try_create_netrc returns the same RAII guard — the file lives while held
    /// and is deleted on drop.
    #[test]
    fn try_create_netrc_file_lives_until_dropped() {
        let auth = test_auth();
        let temp_path = auth
            .try_create_netrc()
            .expect("try_create_netrc should return Some");
        let path = temp_path.to_path_buf();
        assert!(
            path.exists(),
            "netrc file should exist while TempPath is held"
        );
        drop(temp_path);
        assert!(
            !path.exists(),
            "netrc file should be deleted when TempPath is dropped"
        );
    }

    /// try_create_netrc returns None when no token is present.
    #[test]
    fn try_create_netrc_returns_none_without_token() {
        let auth = NixAuth::from_tempdir_and_context(tempdir().unwrap(), AuthContext::Auth0(None));
        assert!(auth.try_create_netrc().is_none());
    }
}
