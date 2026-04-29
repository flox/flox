//! Authentication context types.
//!
//! [`AuthContext`] is the central authentication type threaded through the CLI.
//! It captures both the *kind* of authentication in use (Auth0 / Kerberos) and
//! the authentication material available for that kind (which may be absent —
//! e.g. no token yet, or no Kerberos ticket).
//!
//! Transport layers (HTTP catalog client, git credential helper) inspect the
//! variant to decide how to authenticate requests. "No material" is an
//! explicit state rather than a separate variant so that the configured auth
//! mode is always preserved.

use std::fmt;
use std::sync::Arc;

use url::Url;

use crate::token::FloxhubToken;

/// Describes why authentication failed and whether interactive recovery is possible.
#[derive(Debug, Clone)]
pub struct AuthFailure {
    /// User-facing description of why authentication failed.
    pub message: String,
    /// Whether the CLI's interactive login flow can recover from this failure.
    pub recoverable: bool,
}

impl fmt::Display for AuthFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// A function that generates a SPNEGO token for a given URL.
pub type TokenGenerator = Arc<dyn Fn(&Url) -> Result<String, String> + Send + Sync>;

/// Material for Kerberos authentication.
#[derive(Clone)]
pub struct KerberosMaterial {
    /// The resolved principal name.
    pub principal: String,
    /// A function to generate SPNEGO tokens.
    pub generate_token: TokenGenerator,
}

/// Authentication context threaded through the CLI.
///
/// Each variant corresponds to a configured [`AuthnMode`](super::AuthnMode)
/// and wraps an `Option` of the material for that mode:
///
/// - `Auth0(Some(token))` — logged in via Auth0, token may or may not be
///   expired (checked lazily).
/// - `Auth0(None)` — Auth0 mode but no token yet (not logged in).
/// - `Kerberos(Some(material))` — Kerberos mode with a resolved principal
///   and SPNEGO token generator.
/// - `Kerberos(None)` — Kerberos mode but no ticket available (`kinit`
///   hasn't been run).
///
/// Transport adapters match on the variant to decide how to authenticate:
/// the HTTP catalog client calls [`authorization_header`](Self::authorization_header)
/// to get a bearer or Negotiate header, while the git credential helper
/// uses the variant to decide between an inline credential helper and a
/// no-op (kerberized git authenticates via the ccache directly).
#[derive(Clone)]
pub enum AuthContext {
    /// Auth0 authentication — may or may not have a token.
    Auth0(Option<FloxhubToken>),
    /// Kerberos authentication — may or may not have a ticket/principal.
    Kerberos(Option<KerberosMaterial>),
}

impl AuthContext {
    /// Return the user's handle/identity, if available.
    pub fn handle(&self) -> Option<&str> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(token.handle()),
            AuthContext::Auth0(None) => None,
            AuthContext::Kerberos(Some(material)) => Some(&material.principal),
            AuthContext::Kerberos(None) => None,
        }
    }

    /// Return the user's handle if authenticated, or an [`AuthFailure`]
    /// describing why authentication failed.
    pub fn authenticated_handle(&self) -> Result<String, AuthFailure> {
        match self {
            AuthContext::Auth0(Some(token)) if token.is_expired() => Err(AuthFailure {
                message: "Your FloxHub token has expired.".to_string(),
                recoverable: true,
            }),
            AuthContext::Auth0(Some(token)) => Ok(token.handle().to_string()),
            AuthContext::Auth0(None) => Err(AuthFailure {
                message: "You are not logged in to FloxHub.".to_string(),
                recoverable: true,
            }),
            AuthContext::Kerberos(Some(material)) => Ok(material.principal.clone()),
            AuthContext::Kerberos(None) => Err(AuthFailure {
                message: "Kerberos ticket not found. Run 'kinit' to authenticate.".to_string(),
                recoverable: false,
            }),
        }
    }

    /// Produce the value for an HTTP Authorization header targeting the given URL.
    pub fn authorization_header(&self, url: &Url) -> Option<Result<String, String>> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(Ok(format!("bearer {}", token.secret()))),
            AuthContext::Auth0(None) => None,
            AuthContext::Kerberos(Some(material)) => {
                Some((material.generate_token)(url).map(|t| format!("Negotiate {t}")))
            },
            AuthContext::Kerberos(None) => None,
        }
    }
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthContext::Auth0(Some(_)) => f.debug_tuple("Auth0").field(&"<token>").finish(),
            AuthContext::Auth0(None) => f.write_str("Auth0(None)"),
            AuthContext::Kerberos(Some(material)) => f
                .debug_struct("Kerberos")
                .field("principal", &material.principal)
                .finish_non_exhaustive(),
            AuthContext::Kerberos(None) => f.write_str("Kerberos(None)"),
        }
    }
}
