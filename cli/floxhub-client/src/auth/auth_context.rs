//! [`AuthContext`] — the credential threaded through the CLI.
//!
//! [`AuthContext`] is the central authentication type threaded through the CLI.
//! It captures both the *kind* of credential in use — decided by what the
//! credential answers locally: Auth0-shaped JWT, bare JWT, opaque token, or
//! Kerberos — and the material available for that kind (which may be
//! absent — e.g. no token yet, or no Kerberos ticket).
//!
//! Transport layers (HTTP catalog client, git credential helper) inspect the
//! variant to decide how to authenticate requests. "No material" is an
//! explicit state rather than a separate variant so that the configured auth
//! mode is always preserved.

use url::Url;

use crate::auth::kerberos::KerberosMaterial;
use crate::auth::token::{ACCESS_TOKEN_PREFIX, AccessToken, BareToken, FloxhubToken};

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
/// - `Auth0(Some(token))` — an Auth0-shaped JWT, identity answered from
///   its claims; the token may or may not be expired (checked lazily).
/// - `Auth0(None)` — interactive-login mode but no token yet (not logged
///   in).
/// - `Bare(token)` — a decodable JWT without the handle claim (an issuer
///   other than the Auth0 tenant, e.g. a deployment's Dex); `exp` and
///   `sub` read from the claims, identity resolved at the point of use
///   and cached process-wide.
/// - `AccessToken(token)` — not decodable at all: a `flox_`-prefixed
///   token (e.g. a `flox_pat_` personal access token) or any opaque
///   string an issuer mints; identity is resolved at the point of use
///   and cached process-wide.
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
    /// Auth0-shaped JWT — identity answered locally from its claims.
    /// May or may not have a token; the settled server-side direction is
    /// that identity comes from accounts, so this is the shape being
    /// retired as issuers stop emitting the handle claim.
    Auth0(Option<FloxhubToken>),
    /// Decodable JWT without the handle claim — identity resolved lazily
    /// via /me and cached process-wide. No `Option`: "logged-in mode with
    /// no token" remains `Auth0(None)`.
    Bare(BareToken),
    /// Opaque token (`flox_`-prefixed, or any string that doesn't decode
    /// as a JWT) — identity is resolved lazily and cached process-wide.
    /// No `Option`, as for `Bare`.
    AccessToken(AccessToken),
    /// Kerberos authentication — may or may not have a ticket/principal.
    Kerberos(Option<KerberosMaterial>),
}

impl AuthContext {
    /// Return the user's handle, when it is known locally: JWT claims, a
    /// Kerberos principal, or a token whose identity was already resolved
    /// and cached. Never blocks and never touches the network — for the
    /// resolved answer use `Flox::get_identity`.
    pub fn handle(&self) -> Option<String> {
        match self {
            AuthContext::Auth0(Some(token)) => Some(token.handle().to_string()),
            AuthContext::Auth0(None) => None,
            AuthContext::Bare(token) => token.handle(),
            AuthContext::AccessToken(token) => token.handle(),
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
    /// [`FloxhubToken::sub`]: crate::auth::token::FloxhubToken::sub
    pub fn user_subject(&self) -> Option<&str> {
        match self {
            AuthContext::Auth0(Some(token)) => token.sub(),
            AuthContext::Auth0(None) => None,
            AuthContext::Bare(token) => token.sub(),
            // An opaque token carries no locally readable subject.
            AuthContext::AccessToken(_) => None,
            AuthContext::Kerberos(_) => None,
        }
    }

    /// Produce the value for an HTTP Authorization header targeting the given URL.
    pub fn authorization_header(&self, url: &Url) -> Option<Result<String, AuthHeaderError>> {
        match self {
            AuthContext::Auth0(_) | AuthContext::Bare(_) | AuthContext::AccessToken(_) => self
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
            AuthContext::Bare(token) => Some(token.secret()),
            AuthContext::AccessToken(token) => Some(token.secret()),
            AuthContext::Kerberos(_) => None,
        }
    }

    /// Create an [`AuthContext`] from a stored token, routing by what the
    /// credential's claims answer locally:
    ///
    /// - `flox_`-prefixed token: [`AuthContext::AccessToken`] — opaque by
    ///   fiat, never decoded.
    /// - Auth0-shaped JWT (handle claim and expiry): [`AuthContext::Auth0`].
    /// - Any other decodable JWT: [`AuthContext::Bare`].
    /// - Anything else: [`AuthContext::AccessToken`] — an issuer may mint
    ///   opaque access tokens.
    /// - No token: `Auth0(None)` (not logged in).
    ///
    /// Routing is total: no local check can reject a token, and the
    /// server's 401 is the authority on validity.
    pub fn new_from_token(token: Option<&str>) -> Self {
        let Some(token) = token else {
            return AuthContext::Auth0(None);
        };
        if token.starts_with(ACCESS_TOKEN_PREFIX) {
            return AuthContext::AccessToken(AccessToken::new(token.to_string()));
        }
        match FloxhubToken::new(token.to_string()) {
            Ok(parsed) => AuthContext::Auth0(Some(parsed)),
            Err(_) => match BareToken::new(token.to_string()) {
                Ok(parsed) => AuthContext::Bare(parsed),
                Err(_) => AuthContext::AccessToken(AccessToken::new(token.to_string())),
            },
        }
    }

    /// Returns true when no valid authentication material is available and a
    /// future catalog-auth-gated call would fail.
    ///
    /// `Auth0(None)` (not logged in) and `Auth0(Some(expired))` both count —
    /// neither carries a credential that will pass once gating is enforced —
    /// and a bare token counts exactly when its exp claim has passed.
    /// `Kerberos(..)` does not require a FloxHub login, `AccessToken` is
    /// opaque (validity unknown until resolved via /me), and a bare token
    /// without the exp claim is equally unknowable, so none of those are
    /// treated as unauthenticated here.
    pub fn is_unauthenticated(&self) -> bool {
        match self {
            AuthContext::Auth0(None) => true,
            AuthContext::Auth0(Some(token)) => token.is_expired(),
            AuthContext::Bare(token) => token.is_expired(),
            AuthContext::AccessToken(_) | AuthContext::Kerberos(_) => false,
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
            AuthContext::Bare(token) => f.debug_tuple("Bare").field(&token).finish(),
            AuthContext::AccessToken(token) => f.debug_tuple("AccessToken").field(&token).finish(),
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
    use crate::auth::token::FloxhubToken;
    use crate::auth::token::test_helpers::{
        FAKE_EXPIRED_TOKEN_WITH_SUB,
        FAKE_TOKEN,
        FAKE_TOKEN_NO_HANDLE,
        FAKE_TOKEN_WITH_SUB,
        test_bare_token,
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

    #[test]
    fn is_unauthenticated_covers_missing_and_expired_auth0_tokens() {
        let valid = FloxhubToken::from_str(FAKE_TOKEN).expect("token parses");
        let expired = FloxhubToken::from_str(FAKE_EXPIRED_TOKEN_WITH_SUB).expect("token parses");
        assert!(expired.is_expired(), "test premise: token is expired");

        assert!(AuthContext::Auth0(None).is_unauthenticated());
        assert!(AuthContext::Auth0(Some(expired)).is_unauthenticated());
        assert!(!AuthContext::Auth0(Some(valid)).is_unauthenticated());
        assert!(!pat_unresolved().is_unauthenticated());
        assert!(!AuthContext::Kerberos(None).is_unauthenticated());
    }

    fn pat_unresolved() -> AuthContext {
        AuthContext::AccessToken(AccessToken::new("flox_pat_secret".to_string()))
    }

    #[test]
    fn pat_handle_is_unknown_until_resolved() {
        let auth = pat_unresolved();
        assert_eq!(auth.handle(), None);
    }

    #[test]
    fn pat_handle_reads_the_cached_identity() {
        let token = AccessToken::new("flox_pat_context-handle-test".to_string());
        crate::auth::identity::cache_identity(token.secret(), &test_identity("testuser"));
        let auth = AuthContext::AccessToken(token);

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
    fn new_from_token_routes_flox_prefix_to_access_token() {
        // Any flox_-prefixed token is an opaque access token: personal
        // access tokens today, service account tokens to come.
        for secret in ["flox_pat_abc123", "flox_sat_abc123"] {
            let auth = AuthContext::new_from_token(Some(secret));
            let AuthContext::AccessToken(token) = auth else {
                panic!("expected AccessToken, got {auth:?}");
            };
            assert_eq!(token.secret(), secret);
        }
    }

    #[test]
    fn new_from_token_routes_jwt_to_auth0() {
        let auth = AuthContext::new_from_token(Some(FAKE_TOKEN));
        let AuthContext::Auth0(Some(token)) = auth else {
            panic!("expected Auth0, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN);
    }

    #[test]
    fn new_from_token_without_token_is_not_logged_in() {
        let auth = AuthContext::new_from_token(None);
        assert!(matches!(auth, AuthContext::Auth0(None)));
    }

    #[test]
    fn new_from_token_routes_claimless_jwt_to_bare() {
        let auth = AuthContext::new_from_token(Some(FAKE_TOKEN_NO_HANDLE));
        let AuthContext::Bare(token) = auth else {
            panic!("expected Bare, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN_NO_HANDLE);
    }

    #[test]
    fn new_from_token_carries_a_non_jwt_opaquely() {
        // No local check can reject a token — an issuer may mint opaque
        // access tokens, so a non-decodable string is a credential whose
        // validity only the server can judge.
        let auth = AuthContext::new_from_token(Some("not-a-jwt"));
        let AuthContext::AccessToken(token) = auth else {
            panic!("expected AccessToken, got {auth:?}");
        };
        assert_eq!(token.secret(), "not-a-jwt");
    }

    #[test]
    fn bare_token_subject_and_cached_handle() {
        // A bare token contributes its sub for telemetry, and its handle
        // comes from the /me-filled cache, like an opaque token's.
        let token = test_bare_token("context-bare-handle-test");
        let auth = AuthContext::Bare(token.clone());
        assert_eq!(auth.user_subject(), Some("context-bare-handle-test"));
        assert_eq!(auth.handle(), None);

        crate::auth::identity::cache_identity(token.secret(), &test_identity("dexter"));
        assert_eq!(
            AuthContext::Bare(token).handle(),
            Some("dexter".to_string())
        );
    }
}
