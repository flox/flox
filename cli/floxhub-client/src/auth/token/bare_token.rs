//! [`BareToken`] — a decodable JWT that does not identify its owner.
//!
//! Issuers other than the Auth0 tenant (a deployment's Dex) mint tokens
//! without the FloxHub handle claim, and the settled server-side model
//! trusts the provider for `sub` only, with the handle coming from
//! accounts. A bare token therefore reads what its claims do carry —
//! `exp` for the local expiry warning, `sub` for telemetry attribution —
//! and resolves its identity from `/me` at the point of use, cached
//! process-wide exactly like an opaque [`AccessToken`]'s.
//!
//! [`AccessToken`]: crate::auth::token::AccessToken

use serde::Deserialize;

use crate::auth::identity;
use crate::auth::token::InvalidTokenError;

/// The claims a bare token may carry; every one is optional — a JWT with
/// none of them is still a valid credential.
#[derive(Debug, Clone, Default, Deserialize)]
struct BareTokenClaims {
    /// The expiration time of the token (Unix timestamp)
    exp: Option<usize>,
    /// The OIDC subject identifier — an opaque, pseudonymous id stable
    /// across the user's lifetime. Declaring the claim here means a
    /// non-string `sub` fails the decode — deliberate, since OIDC
    /// requires `sub` to be a string.
    sub: Option<String>,
}

/// A bearer credential that is a JWT but carries no FloxHub identity;
/// the identity resolves from `/me` and is cached, like an opaque
/// token's.
#[derive(Clone)]
pub struct BareToken {
    /// The entire token as a string
    token: String,
    claims: BareTokenClaims,
}

impl BareToken {
    /// Decode a JWT without requiring any identity claims. Errors when
    /// the string is not a decodable JWT at all — such a credential is
    /// opaque, not bare.
    pub fn new(token: String) -> Result<Self, InvalidTokenError> {
        let decoded = jsonwebtoken::dangerous::insecure_decode::<BareTokenClaims>(&token)
            .map_err(InvalidTokenError)?;

        Ok(BareToken {
            token,
            claims: decoded.claims,
        })
    }

    /// Return the token as a string
    pub fn secret(&self) -> &str {
        &self.token
    }

    /// Return the resolved handle; `None` until `/me` resolution has
    /// succeeded. Reads the process-wide cache — never blocks, never
    /// touches the network. For the resolved answer use
    /// `Flox::get_identity`.
    pub fn handle(&self) -> Option<String> {
        identity::cached_identity(&self.token).map(|identity| identity.handle)
    }

    /// Return the OIDC `sub` claim, if present and non-empty. See
    /// [`FloxhubToken::sub`](crate::auth::token::FloxhubToken::sub).
    pub fn sub(&self) -> Option<&str> {
        self.claims.sub.as_deref().filter(|s| !s.is_empty())
    }

    /// Returns whether the token has expired by checking the `exp` claim
    /// against the current time. A token without the claim never expires
    /// locally — the server's 401 is the authority.
    pub fn is_expired(&self) -> bool {
        let Some(exp) = self.claims.exp else {
            return false;
        };
        let now = {
            let start = std::time::SystemTime::now();
            start
                .duration_since(std::time::UNIX_EPOCH)
                .expect("Time went backwards")
                .as_secs() as usize
        };
        exp < now
    }

    /// The wall-clock expiry of the token, from the `exp` claim;
    /// `None` when the token doesn't carry one.
    pub fn expires_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.claims.exp.map(|exp| {
            chrono::DateTime::from_timestamp(exp as i64, 0)
                .expect("the exp claim is a valid unix timestamp")
        })
    }
}

impl std::fmt::Debug for BareToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BareToken")
            .field("claims", &self.claims)
            .field("identity", &identity::cached_identity(&self.token))
            .finish_non_exhaustive()
    }
}

/// Test fixtures for [BareToken].
///
/// Nothing here should be used in production code.
#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    use super::*;

    /// A fake token shaped like a Dex-issued one: `sub` and `exp` but no
    /// handle claim.
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    ///   "sub": "CiQwOGE4Njg0Yi1kYjg4LTRiNzMtOTBhOS0zY2QxNjYxZjU0NjYSBWxvY2Fs",
    ///   "email": "dev@flox.dev",
    ///   "name": "dexter"
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    pub const FAKE_TOKEN_NO_HANDLE: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJleHAiOjk5OTk5OTk5OTksInN1YiI6IkNpUXdPR0U0TmpnMFlpMWtZamc0TFRSaU56TXRPVEJoT1MwelkyUXhOall4WmpVME5qWVNCV3h2WTJGcyIsImVtYWlsIjoiZGV2QGZsb3guZGV2IiwibmFtZSI6ImRleHRlciJ9.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    /// A bare token with a unique `sub`, for tests that touch the
    /// process-wide identity cache and must not share a secret with any
    /// other test.
    pub fn test_bare_token(sub: &str) -> BareToken {
        let claims = serde_json::json!({ "sub": sub, "exp": 9999999999_i64 });
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &claims,
            &jsonwebtoken::EncodingKey::from_secret("secret".as_ref()),
        )
        .expect("encoding a test JWT succeeds");
        BareToken::new(token).expect("a freshly encoded JWT decodes")
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::*;
    use super::*;
    use crate::auth::identity::test_helpers::test_identity;

    #[test]
    fn dex_shaped_jwt_parses_with_claims_but_no_handle() {
        let token = BareToken::new(FAKE_TOKEN_NO_HANDLE.to_string()).expect("token parses");
        assert_eq!(token.handle(), None, "handle is unknown before resolution");
        assert_eq!(
            token.sub(),
            Some("CiQwOGE4Njg0Yi1kYjg4LTRiNzMtOTBhOS0zY2QxNjYxZjU0NjYSBWxvY2Fs")
        );
        assert!(!token.is_expired());
        assert!(token.expires_at().is_some());
    }

    #[test]
    fn non_jwt_is_rejected() {
        BareToken::new("not-a-jwt".to_string()).unwrap_err();
    }

    #[test]
    fn handle_reads_the_identity_cache() {
        let token = test_bare_token("bare-handle-cache-test");
        assert_eq!(token.handle(), None, "handle is unknown before resolution");

        identity::cache_identity(token.secret(), &test_identity("bareuser"));
        assert_eq!(token.handle(), Some("bareuser".to_string()));
    }

    #[test]
    fn debug_redacts_the_secret() {
        let token = test_bare_token("bare-debug-test");
        assert!(!format!("{token:?}").contains(token.secret()));
    }
}
