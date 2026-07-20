//! [`AuthContext`] — the credential threaded through the CLI.
//!
//! [`AuthContext`] is the central authentication type threaded through the CLI.
//! It captures both the *kind* of authentication in use (Auth0 / PAT /
//! Kerberos) and the authentication material available for that kind (which
//! may be absent — e.g. no token yet, or no Kerberos ticket).
//!
//! Transport layers (HTTP catalog client, git credential helper) inspect the
//! variant to decide how to authenticate requests. "No material" is an
//! explicit state rather than a separate variant so that the configured auth
//! mode is always preserved.

use url::Url;

use crate::auth::kerberos::KerberosMaterial;
use crate::auth::token::{FloxhubToken, FloxhubTokenError, PAT_PREFIX, PersonalAccessToken};

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

/// Authentication context threaded through the CLI.
///
/// Each variant corresponds to a kind of authentication and wraps an
/// `Option` of the material for that kind:
///
/// - `Auth0(Some(token))` — logged in via Auth0, token may or may not be
///   expired (checked lazily).
/// - `Auth0(None)` — Auth0 mode but no token yet (not logged in).
/// - `Pat(token)` — personal access token (`flox_pat_…`); identity is
///   resolved at the point of use and cached on the token.
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
    /// lazily and cached process-wide. No `Option`: "Auth0 mode with no
    /// token" remains `Auth0(None)`.
    Pat(PersonalAccessToken),
    /// Kerberos authentication — may or may not have a ticket/principal.
    Kerberos(Option<KerberosMaterial>),
}

impl AuthContext {
    /// Return the user's handle, when it is known locally: JWT claims, a
    /// Kerberos principal, or an opaque token whose identity was already
    /// resolved and cached. Never blocks and never touches the network —
    /// for the resolved answer use `Flox::get_identity`.
    pub fn handle(&self) -> Option<String> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(token.handle().to_string()),
            AuthContext::Auth0(None) => None,
            AuthContext::Pat(token) => token.handle(),
            AuthContext::Kerberos(Some(material)) => Some(material.principal.clone()),
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

    /// Create an [`AuthContext`] from a stored token, routing by the
    /// token's form:
    ///
    /// - `flox_pat_` token: [`AuthContext::Pat`] — the token stays opaque;
    ///   its identity is resolved at the point of use.
    /// - Any other token: must decode as a JWT → [`AuthContext::Auth0`].
    /// - No token: `Auth0(None)` (not logged in).
    pub fn new_from_token(token: Option<&str>) -> Result<Self, FloxhubTokenError> {
        match token {
            Some(token) if token.starts_with(PAT_PREFIX) => Ok(AuthContext::Pat(
                PersonalAccessToken::new(token.to_string()),
            )),
            Some(token) => Ok(AuthContext::Auth0(Some(token.parse()?))),
            None => Ok(AuthContext::Auth0(None)),
        }
    }

    /// Create a Kerberos [`AuthContext`]: resolves the principal and embeds
    /// a SPNEGO token generator; returns `Kerberos(None)` (with a warning
    /// log) if the ticket cannot be resolved. FloxHub tokens are not used.
    pub fn new_kerberos() -> Self {
        crate::auth::kerberos::kerberos_credential()
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

    use super::*;
    use crate::auth::identity::test_helpers::test_identity;
    use crate::auth::token::test_helpers::{
        FAKE_EXPIRED_TOKEN_WITH_SUB,
        FAKE_TOKEN,
        FAKE_TOKEN_WITH_SUB,
    };
    use crate::auth::token::FloxhubToken;

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

    fn pat_unresolved() -> AuthContext {
        AuthContext::Pat(PersonalAccessToken::new("flox_pat_secret".to_string()))
    }

    #[test]
    fn pat_handle_is_unknown_until_resolved() {
        let auth = pat_unresolved();
        assert_eq!(auth.handle(), None);
    }

    #[test]
    fn pat_handle_reads_the_cached_identity() {
        let token = PersonalAccessToken::new("flox_pat_context-handle-test".to_string());
        crate::auth::identity::cache_identity(token.secret(), &test_identity("testuser"));
        let auth = AuthContext::Pat(token);

        assert_eq!(auth.handle(), Some("testuser".to_string()));
    }

    #[test]
    fn pat_authorization_header_is_bearer_secret() {
        let auth = pat_unresolved();
        let url = Url::parse("https://api.flox.dev").unwrap();

        assert_eq!(
            auth.authorization_header(&url).unwrap().unwrap(),
            "bearer flox_pat_secret"
        );
    }

    #[test]
    fn pat_debug_redacts_the_secret() {
        let auth = pat_unresolved();
        assert!(!format!("{auth:?}").contains("flox_pat_secret"));
    }

    #[test]
    fn jwt_handle_derives_from_claims() {
        let auth = AuthContext::Auth0(Some(FAKE_TOKEN.parse().unwrap()));
        assert_eq!(auth.handle(), Some("test".to_string()));
    }

    #[test]
    fn new_from_token_routes_pat_prefix_to_pat() {
        let auth = AuthContext::new_from_token(Some("flox_pat_abc123")).unwrap();
        let AuthContext::Pat(token) = auth else {
            panic!("expected Pat, got {auth:?}");
        };
        assert_eq!(token.secret(), "flox_pat_abc123");
    }

    #[test]
    fn new_from_token_routes_jwt_to_auth0() {
        let auth = AuthContext::new_from_token(Some(FAKE_TOKEN)).unwrap();
        let AuthContext::Auth0(Some(token)) = auth else {
            panic!("expected Auth0, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN);
    }

    #[test]
    fn new_from_token_without_token_is_not_logged_in() {
        let auth = AuthContext::new_from_token(None).unwrap();
        assert!(matches!(auth, AuthContext::Auth0(None)));
    }

    #[test]
    fn new_from_token_rejects_garbage() {
        AuthContext::new_from_token(Some("not-a-token")).unwrap_err();
    }
}
