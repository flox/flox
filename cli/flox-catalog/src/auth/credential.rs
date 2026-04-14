//! Authentication credential types

use std::sync::Arc;

use url::Url;

use super::AuthMethod;
use crate::token::FloxhubToken;

/// A function that generates a SPNEGO token for a given URL.
pub type TokenGenerator = Arc<dyn Fn(&Url) -> Result<String, String> + Send + Sync>;

/// Represents available authentication material.
/// Transport adapters decide how to apply it.
#[derive(Clone)]
pub enum Credential {
    /// A bearer token (JWT from Auth0)
    Bearer(FloxhubToken),
    /// Kerberos — carries the resolved principal and a function to generate
    /// SPNEGO tokens for a target URL. Git transport ignores the token generator
    /// (kerberized git uses the ccache directly).
    Kerberos {
        principal: String,
        generate_token: TokenGenerator,
    },
    /// No credential available.
    None,
}

impl Credential {
    /// Return the user's handle/identity, if available.
    pub fn handle(&self) -> Option<String> {
        match self {
            Credential::Bearer(token) => Some(token.handle().to_string()),
            Credential::Kerberos { principal, .. } => Some(principal.clone()),
            Credential::None => None,
        }
    }

    /// Whether the credential is expired or missing.
    ///
    /// - Bearer: checks JWT expiration
    /// - Kerberos: always false (ticket validity is managed externally via `kinit`)
    /// - None: always true (no credential = needs authentication)
    pub fn is_expired(&self) -> bool {
        match self {
            Credential::Bearer(token) => token.is_expired(),
            Credential::Kerberos { .. } => false,
            Credential::None => true,
        }
    }

    /// Return the authentication method this credential corresponds to.
    pub fn auth_method(&self) -> AuthMethod {
        match self {
            #[cfg(feature = "floxhub-authn-kerberos")]
            Credential::Kerberos { .. } => AuthMethod::Kerberos,
            _ => AuthMethod::Auth0,
        }
    }

    /// Produce the value for an HTTP Authorization header targeting the given URL.
    pub fn authorization_header(&self, url: &Url) -> Option<Result<String, String>> {
        match self {
            Credential::Bearer(token) => Some(Ok(format!("bearer {}", token.secret()))),
            Credential::Kerberos { generate_token, .. } => {
                Some(generate_token(url).map(|t| format!("Negotiate {t}")))
            },
            Credential::None => None,
        }
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Bearer(_) => f.debug_tuple("Bearer").field(&"<token>").finish(),
            Credential::Kerberos { principal, .. } => f
                .debug_struct("Kerberos")
                .field("principal", principal)
                .finish_non_exhaustive(),
            Credential::None => f.write_str("None"),
        }
    }
}
