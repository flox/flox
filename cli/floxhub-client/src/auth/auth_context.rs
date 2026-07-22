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

use std::sync::Arc;

use url::Url;

use crate::token::FloxhubToken;

/// Describes why authentication failed.
///
/// The CLI layer decides how to present these failures to the user and whether
/// interactive recovery is possible.
#[derive(Debug, Clone, thiserror::Error)]
pub enum AuthFailure {
    /// Auth0 token exists but has expired.
    #[error("token expired")]
    TokenExpired,
    /// Auth0 mode but no token is available.
    #[error("not logged in")]
    NotLoggedIn,
    /// Kerberos mode but no ticket is available.
    #[error("no kerberos ticket")]
    NoKerberosTicket,
}

/// Error from producing an authorization header (e.g. SPNEGO token generation).
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct AuthHeaderError(pub String);

/// A function that generates a SPNEGO token for a given URL.
pub type TokenGenerator = Arc<dyn Fn(&Url) -> Result<String, AuthHeaderError> + Send + Sync>;

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

    /// Return the pseudonymous subject identifier for telemetry
    /// attribution, if one is available.
    ///
    /// Auth0 tokens carry the OIDC `sub` claim ([`FloxhubToken::sub`]) —
    /// opaque and stable across the user's lifetime, so it remains valid
    /// attribution even when the token has expired. Kerberos has no
    /// pseudonymous equivalent today (the principal is directly
    /// identifying), so kerberos-mode invocations return `None`.
    ///
    /// [`FloxhubToken::sub`]: crate::token::FloxhubToken::sub
    pub fn user_subject(&self) -> Option<&str> {
        match self {
            AuthContext::Auth0(Some(token)) => token.sub(),
            AuthContext::Auth0(None) => None,
            AuthContext::Kerberos(_) => None,
        }
    }

    /// Return the user's handle if authenticated, or an [`AuthFailure`]
    /// describing why authentication failed.
    pub fn authenticated_handle(&self) -> Result<&str, AuthFailure> {
        match self {
            AuthContext::Auth0(Some(token)) if token.is_expired() => Err(AuthFailure::TokenExpired),
            AuthContext::Auth0(Some(token)) => Ok(token.handle()),
            AuthContext::Auth0(None) => Err(AuthFailure::NotLoggedIn),
            AuthContext::Kerberos(Some(material)) => Ok(&material.principal),
            AuthContext::Kerberos(None) => Err(AuthFailure::NoKerberosTicket),
        }
    }

    /// Return the user's handle if authenticated, allowing expired auth0 tokens,
    /// or an [`AuthFailure`] describing why authentication failed.
    pub fn authenticated_handle_allowing_expired(&self) -> Result<&str, AuthFailure> {
        match self {
            AuthContext::Auth0(Some(token)) => Ok(token.handle()),
            AuthContext::Auth0(None) => Err(AuthFailure::NotLoggedIn),
            AuthContext::Kerberos(Some(material)) => Ok(&material.principal),
            AuthContext::Kerberos(None) => Err(AuthFailure::NoKerberosTicket),
        }
    }

    /// Produce the value for an HTTP Authorization header targeting the given URL.
    pub fn authorization_header(&self, url: &Url) -> Option<Result<String, AuthHeaderError>> {
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::token::FloxhubToken;
    use crate::token::test_helpers::{
        FAKE_EXPIRED_TOKEN_WITH_SUB,
        FAKE_TOKEN,
        FAKE_TOKEN_WITH_SUB,
    };

    #[test]
    fn user_subject_returns_sub_for_auth0_token() {
        let token = FloxhubToken::from_str(FAKE_TOKEN_WITH_SUB).expect("token parses");
        assert_eq!(
            AuthContext::Auth0(Some(token)).user_subject(),
            Some("github|424242")
        );
    }

    /// Expiry gates authentication, not identity — an expired token's `sub`
    /// is still the correct attribution.
    #[test]
    fn user_subject_returns_sub_for_expired_auth0_token() {
        let token = FloxhubToken::from_str(FAKE_EXPIRED_TOKEN_WITH_SUB).expect("token parses");
        assert!(token.is_expired(), "test premise: token is expired");
        assert_eq!(
            AuthContext::Auth0(Some(token)).user_subject(),
            Some("github|424242")
        );
    }

    #[test]
    fn user_subject_is_none_without_sub_token_or_auth0() {
        let token = FloxhubToken::from_str(FAKE_TOKEN).expect("token parses");
        assert_eq!(AuthContext::Auth0(Some(token)).user_subject(), None);
        assert_eq!(AuthContext::Auth0(None).user_subject(), None);
        assert_eq!(AuthContext::Kerberos(None).user_subject(), None);
    }
}
