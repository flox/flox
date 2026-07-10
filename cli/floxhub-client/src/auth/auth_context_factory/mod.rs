//! AuthContext construction from the configured auth method.

use crate::auth::{AuthContext, AuthnMode};
use crate::token::{FloxhubTokenError, PAT_PREFIX, PersonalAccessToken};

mod kerberos;

impl AuthContext {
    /// Create an [`AuthContext`] from the configured [`AuthnMode`] and the
    /// stored token, both of which are needed to pick the right material:
    ///
    /// - Auth0 + `flox_pat_` token: [`AuthContext::Pat`] — the token stays
    ///   opaque and its identity is resolved lazily via `/me`.
    /// - Auth0 + any other token: must decode as a JWT →
    ///   [`AuthContext::Auth0`].
    /// - Auth0 + no token: `Auth0(None)` (not logged in).
    /// - Kerberos: resolves the principal and embeds a SPNEGO token
    ///   generator; returns `Kerberos(None)` (with a warning log) if the
    ///   ticket cannot be resolved. The token is not consumed.
    ///
    /// `api_url` is the FloxHub API base an opaque token resolves its
    /// identity against; the other materials do not use it.
    pub fn from_mode(
        mode: &AuthnMode,
        token: Option<&str>,
        api_url: &str,
    ) -> Result<Self, FloxhubTokenError> {
        match mode {
            AuthnMode::Auth0 => match token {
                Some(token) if token.starts_with(PAT_PREFIX) => Ok(AuthContext::Pat(
                    PersonalAccessToken::new(token.to_string(), api_url.to_string()),
                )),
                Some(token) => Ok(AuthContext::Auth0(Some(token.parse()?))),
                None => Ok(AuthContext::Auth0(None)),
            },
            AuthnMode::Kerberos => Ok(kerberos::kerberos_credential()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A structurally valid JWT with the flox handle claim, signed with a
    /// throwaway key (signatures are not verified client side).
    fn make_jwt(handle: &str, exp: i64) -> String {
        let claims = serde_json::json!({
            "https://flox.dev/handle": handle,
            "exp": exp,
        });
        jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret("test".as_bytes()),
        )
        .unwrap()
    }

    #[test]
    fn from_mode_routes_pat_prefix_to_pat() {
        let auth = AuthContext::from_mode(
            &AuthnMode::Auth0,
            Some("flox_pat_abc123"),
            "https://not_used",
        )
        .unwrap();
        let AuthContext::Pat(token) = auth else {
            panic!("expected Pat, got {auth:?}");
        };
        assert_eq!(token.secret(), "flox_pat_abc123");
    }

    #[test]
    fn from_mode_routes_jwt_to_auth0() {
        let jwt = make_jwt("testuser", 9999999999);
        let auth =
            AuthContext::from_mode(&AuthnMode::Auth0, Some(&jwt), "https://not_used").unwrap();
        let AuthContext::Auth0(Some(token)) = auth else {
            panic!("expected Auth0, got {auth:?}");
        };
        assert_eq!(token.secret(), jwt);
    }

    #[test]
    fn from_mode_without_token_is_not_logged_in() {
        let auth = AuthContext::from_mode(&AuthnMode::Auth0, None, "https://not_used").unwrap();
        assert!(matches!(auth, AuthContext::Auth0(None)));
    }

    #[test]
    fn from_mode_rejects_garbage() {
        AuthContext::from_mode(&AuthnMode::Auth0, Some("not-a-token"), "https://not_used")
            .unwrap_err();
    }
}
