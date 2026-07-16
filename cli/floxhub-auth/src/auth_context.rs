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

use crate::authn_mode::AuthnMode;
use crate::identity::{IdentityError, UserIdentity, lazy_identity};
use crate::kerberos::KerberosMaterial;
use crate::token::{FloxhubToken, FloxhubTokenError, PAT_PREFIX, PersonalAccessToken};

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
/// resolved. The server is the authority for authn/authz, so an unknown
/// handle is a display concern, never an access decision.
pub const UNKNOWN_HANDLE: &str = "UNKNOWN";

/// Error from producing an authorization header (e.g. SPNEGO token generation).
#[derive(Debug, Clone, thiserror::Error)]
#[error("{0}")]
pub struct AuthHeaderError(pub String);

/// Authentication context threaded through the CLI.
///
/// Each variant corresponds to a configured [`AuthnMode`](super::AuthnMode)
/// and wraps an `Option` of the material for that mode:
///
/// - `Auth0(Some(token))` — logged in via Auth0, token may or may not be
///   expired (checked lazily).
/// - `Auth0(None)` — Auth0 mode but no token yet (not logged in).
/// - `Pat(token)` — personal access token (`flox_pat_…`); identity is
///   resolved lazily through the token's injected resolver.
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
    /// lazily and cached in memory. No `Option`: "Auth0 mode with no token"
    /// remains `Auth0(None)`.
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

    /// Return the user's handle if authenticated, or an [`AuthFailure`]
    /// describing why authentication failed.
    ///
    /// For an opaque token this lazily resolves the identity, blocking the
    /// calling thread (cached in memory for the process). A rejected token
    /// (invalid, expired, or revoked) is reported as
    /// [`AuthFailure::TokenExpired`]. Any other resolution failure is not
    /// fatal: the handle degrades to [`UNKNOWN_HANDLE`] — the server remains
    /// the authority for whether the token actually authenticates.
    pub fn authenticated_handle(&self) -> Result<&str, AuthFailure> {
        match self {
            AuthContext::Auth0(Some(token)) if token.is_expired() => Err(AuthFailure::TokenExpired),
            AuthContext::Auth0(Some(token)) => Ok(token.handle()),
            AuthContext::Auth0(None) => Err(AuthFailure::NotLoggedIn),
            AuthContext::Pat(token) => {
                match token.resolve_identity() {
                    Ok(_) => {},
                    Err(IdentityError::Unauthorized) => return Err(AuthFailure::TokenExpired),
                    Err(err) => tracing::debug!("could not resolve identity: {err}"),
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

impl AuthContext {
    /// Create an [`AuthContext`] from the configured [`AuthnMode`] and the
    /// stored token, both of which are needed to pick the right material:
    ///
    /// - Auth0 + `flox_pat_` token: [`AuthContext::Pat`] — the token stays
    ///   opaque and resolves its identity lazily through `resolve`.
    /// - Auth0 + any other token: must decode as a JWT →
    ///   [`AuthContext::Auth0`].
    /// - Auth0 + no token: `Auth0(None)` (not logged in).
    /// - Kerberos: resolves the principal and embeds a SPNEGO token
    ///   generator; returns `Kerberos(None)` (with a warning log) if the
    ///   ticket cannot be resolved. The token is not consumed.
    ///
    /// `resolve_identity` supplies the identity behind an opaque token; it is
    /// bound to the token's secret as a lazy, once-per-process resolution.
    /// The other materials do not use it.
    pub fn from_mode(
        mode: &AuthnMode,
        token: Option<&str>,
        resolve_identity: impl FnOnce(String) -> Result<UserIdentity, IdentityError>
        + Send
        + Sync
        + 'static,
    ) -> Result<Self, FloxhubTokenError> {
        match mode {
            AuthnMode::Auth0 => match token {
                Some(token) if token.starts_with(PAT_PREFIX) => {
                    let secret = token.to_string();
                    let identity = lazy_identity({
                        let secret = secret.clone();
                        move || resolve_identity(secret)
                    });
                    Ok(AuthContext::Pat(PersonalAccessToken::new(secret, identity)))
                },
                Some(token) => Ok(AuthContext::Auth0(Some(token.parse()?))),
                None => Ok(AuthContext::Auth0(None)),
            },
            AuthnMode::Kerberos => Ok(crate::kerberos::kerberos_credential()),
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

    use chrono::{Duration, Utc};

    use super::*;
    use crate::identity::LazyIdentity;
    use crate::identity::test_helpers::{
        static_identity,
        test_identity,
        unauthorized_identity,
        unreachable_identity,
        unreachable_resolve,
    };
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

    fn pat_with(identity: LazyIdentity) -> AuthContext {
        AuthContext::Pat(PersonalAccessToken::new(
            "flox_pat_secret".to_string(),
            identity,
        ))
    }

    #[test]
    fn pat_handle_degrades_when_resolution_fails() {
        let auth = pat_with(unreachable_identity());

        assert_eq!(auth.handle(), None);
        assert_eq!(auth.authenticated_handle().unwrap(), UNKNOWN_HANDLE);
    }

    #[test]
    fn pat_handle_reports_expired_when_rejected() {
        let auth = pat_with(unauthorized_identity());

        assert!(matches!(
            auth.authenticated_handle(),
            Err(AuthFailure::TokenExpired)
        ));
    }

    #[test]
    fn pat_handle_reports_expired_from_resolved_expiry() {
        let identity = UserIdentity {
            expires_at: Some(Utc::now() - Duration::hours(1)),
            ..test_identity("testuser")
        };
        let auth = pat_with(static_identity(identity));

        assert!(matches!(
            auth.authenticated_handle(),
            Err(AuthFailure::TokenExpired)
        ));
        assert!(auth.is_expired());
    }

    #[test]
    fn pat_getters_read_cache_after_resolution() {
        let auth = pat_with(static_identity(test_identity("testuser")));

        assert_eq!(auth.authenticated_handle().unwrap(), "testuser");
        // The identity is now cached; the sync getters see it too.
        assert_eq!(auth.handle(), Some("testuser"));
        assert!(
            !auth.is_expired(),
            "an identity without expiry never expires"
        );
    }

    #[test]
    fn pat_authorization_header_is_bearer_secret() {
        let auth = pat_with(unreachable_identity());
        let url = Url::parse("https://api.flox.dev").unwrap();

        assert_eq!(
            auth.authorization_header(&url).unwrap().unwrap(),
            "bearer flox_pat_secret"
        );
    }

    #[test]
    fn pat_debug_redacts_the_secret() {
        let auth = pat_with(unreachable_identity());
        assert!(!format!("{auth:?}").contains("flox_pat_secret"));
    }

    #[test]
    fn jwt_getters_derive_from_claims() {
        let auth = AuthContext::Auth0(Some(FAKE_TOKEN.parse().unwrap()));

        assert_eq!(auth.handle(), Some("test"));
        assert!(!auth.is_expired());
    }

    #[test]
    fn from_mode_routes_pat_prefix_to_pat() {
        let auth = AuthContext::from_mode(
            &AuthnMode::Auth0,
            Some("flox_pat_abc123"),
            unreachable_resolve,
        )
        .unwrap();
        let AuthContext::Pat(token) = auth else {
            panic!("expected Pat, got {auth:?}");
        };
        assert_eq!(token.secret(), "flox_pat_abc123");
    }

    #[test]
    fn from_mode_routes_jwt_to_auth0() {
        let auth = AuthContext::from_mode(&AuthnMode::Auth0, Some(FAKE_TOKEN), unreachable_resolve)
            .unwrap();
        let AuthContext::Auth0(Some(token)) = auth else {
            panic!("expected Auth0, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN);
    }

    #[test]
    fn from_mode_without_token_is_not_logged_in() {
        let auth = AuthContext::from_mode(&AuthnMode::Auth0, None, unreachable_resolve).unwrap();
        assert!(matches!(auth, AuthContext::Auth0(None)));
    }

    #[test]
    fn from_mode_rejects_garbage() {
        AuthContext::from_mode(&AuthnMode::Auth0, Some("not-a-token"), unreachable_resolve)
            .unwrap_err();
    }
}
