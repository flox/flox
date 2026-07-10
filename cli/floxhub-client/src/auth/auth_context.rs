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

use crate::accounts::MeError;
use crate::token::{FloxhubToken, PersonalAccessToken};

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

/// Placeholder handle for an opaque token whose identity has not been
/// resolved from `/me`. The server is the authority for authn/authz, so an
/// unknown handle is a display concern, never an access decision.
pub const UNKNOWN_HANDLE: &str = "UNKNOWN";

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
/// - `Pat(token)` — personal access token (`flox_pat_…`); identity is
///   resolved lazily via `/me`.
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
    /// Personal access token (`flox_pat_…`) — opaque; identity is resolved
    /// lazily via `GET /api/v1/accounts/me` and cached in memory. No
    /// `Option`: "Auth0 mode with no token" remains `Auth0(None)`.
    Pat(PersonalAccessToken),
    /// Kerberos authentication — may or may not have a ticket/principal.
    Kerberos(Option<KerberosMaterial>),
}

impl AuthContext {
    /// Return the user's handle/identity, if available.
    pub fn handle(&self) -> Option<&str> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(token.handle()),
            AuthContext::Auth0(None) => None,
            AuthContext::Pat(token) => token.handle(),
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
            // An opaque token carries no locally readable subject.
            AuthContext::Pat(_) => None,
            AuthContext::Kerberos(_) => None,
        }
    }

    /// Return the user's handle if authenticated, or an [`AuthFailure`]
    /// describing why authentication failed.
    ///
    /// For an opaque token this lazily resolves the identity from `/me`,
    /// blocking the calling thread (cached in memory for the process). A 401
    /// from `/me` means the token is invalid, expired, or revoked and is
    /// reported as [`AuthFailure::TokenExpired`]. Any other resolution
    /// failure is not fatal: the handle degrades to [`UNKNOWN_HANDLE`] — the
    /// server remains the authority for whether the token actually
    /// authenticates.
    pub fn authenticated_handle(&self) -> Result<&str, AuthFailure> {
        match self {
            AuthContext::Auth0(Some(token)) if token.is_expired() => Err(AuthFailure::TokenExpired),
            AuthContext::Auth0(Some(token)) => Ok(token.handle()),
            AuthContext::Auth0(None) => Err(AuthFailure::NotLoggedIn),
            AuthContext::Pat(token) => {
                match token.resolve_identity() {
                    Ok(_) => {},
                    Err(MeError::Unauthorized) => return Err(AuthFailure::TokenExpired),
                    Err(err) => tracing::debug!("could not resolve identity from /me: {err}"),
                }
                if token.is_expired() {
                    return Err(AuthFailure::TokenExpired);
                }
                Ok(token.handle().unwrap_or(UNKNOWN_HANDLE))
            },
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
            AuthContext::Auth0(_) | AuthContext::Pat(_) => self
                .token_secret()
                .map(|secret| Ok(format!("bearer {secret}"))),
            AuthContext::Kerberos(Some(material)) => {
                Some((material.generate_token)(url).map(|t| format!("Negotiate {t}")))
            },
            AuthContext::Kerberos(None) => None,
        }
    }

    /// Whether the credential is known to have expired.
    ///
    /// `false` when there is nothing to expire: no token, an opaque token
    /// whose identity has not been resolved yet, or Kerberos (ticket
    /// lifetimes are managed externally via 'kinit').
    pub fn is_expired(&self) -> bool {
        match self {
            AuthContext::Auth0(Some(token)) => token.is_expired(),
            AuthContext::Auth0(None) => false,
            AuthContext::Pat(token) => token.is_expired(),
            AuthContext::Kerberos(_) => false,
        }
    }

    /// Return the raw token secret, if this credential carries one.
    ///
    /// Kerberos does not use bearer tokens, so it has no secret.
    pub fn token_secret(&self) -> Option<&str> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(token.secret()),
            AuthContext::Auth0(None) => None,
            AuthContext::Pat(token) => Some(token.secret()),
            AuthContext::Kerberos(_) => None,
        }
    }
}

impl std::fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthContext::Auth0(Some(_)) => f.debug_tuple("Auth0").field(&"<token>").finish(),
            AuthContext::Auth0(None) => f.write_str("Auth0(None)"),
            AuthContext::Pat(token) => f.debug_tuple("Pat").field(&token).finish(),
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

    use httpmock::MockServer;

    use super::*;
    use crate::token::test_helpers::{
        FAKE_EXPIRED_TOKEN_WITH_SUB,
        FAKE_TOKEN,
        FAKE_TOKEN_WITH_SUB,
    };
    use crate::token::{FloxhubToken, PersonalAccessToken};

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

    #[test]
    fn pat_handle_degrades_when_me_is_unreachable() {
        let auth = AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            // Nothing listens on this port: resolution fails, which is not fatal.
            "http://127.0.0.1:1".to_string(),
        ));

        assert_eq!(auth.handle(), None);
        assert_eq!(auth.authenticated_handle().unwrap(), UNKNOWN_HANDLE);
    }

    #[test]
    fn pat_handle_reports_expired_on_401() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(401);
        });

        let auth = AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_revoked".to_string(),
            server.base_url(),
        ));
        assert!(matches!(
            auth.authenticated_handle(),
            Err(AuthFailure::TokenExpired)
        ));
    }

    #[test]
    fn pat_authorization_header_is_bearer_secret() {
        let auth = AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            "https://not_used".to_string(),
        ));
        let url = Url::parse("https://api.flox.dev").unwrap();

        assert_eq!(
            auth.authorization_header(&url).unwrap().unwrap(),
            "bearer flox_pat_secret"
        );
    }

    #[test]
    fn pat_getters_read_cache_after_resolution() {
        let server = MockServer::start();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/api/v1/accounts/me");
            then.status(200).json_body(serde_json::json!({
                "user_id": "auth0|123",
                "handle": "testuser",
                "expires_at": null,
            }));
        });

        let auth = AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            server.base_url(),
        ));

        assert_eq!(auth.authenticated_handle().unwrap(), "testuser");
        // The identity is now cached; the sync getter sees it too.
        assert_eq!(auth.handle(), Some("testuser"));
    }

    #[test]
    fn pat_debug_redacts_the_secret() {
        let auth = AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            "https://not_used".to_string(),
        ));
        assert!(!format!("{auth:?}").contains("flox_pat_secret"));
    }
}
