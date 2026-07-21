//! Narrowly-scoped JWT `sub`-claim extraction for telemetry identity.
//!
//! Provides [`decode_jwt_sub`] — the single helper the v2-events wrapper
//! ([`crate::utils::events`]) uses to lift the OIDC `sub` claim out of the
//! FloxHub auth token and stamp it as `auth_subject` on emitted events.
//!
//! # Trust model
//!
//! The token is decoded **without signature verification**, mirroring
//! [`FloxhubToken`](floxhub_client::FloxhubToken)'s own parsing: the CLI is
//! reading back a token it received from the FloxHub auth service and
//! stamping a claim onto its own outgoing telemetry — it is not
//! authenticating to anything with the extracted value, so a forged claim
//! would only tamper with the user's own attribution. Skipping verification
//! also means **expired tokens still decode**, which is desired: the `sub`
//! claim is stable across the user's lifetime, so an expired token's `sub`
//! remains the correct attribution even when the token can no longer
//! authenticate new FloxHub API calls.
//!
//! Not reusing [`FloxhubToken`](floxhub_client::FloxhubToken) itself:
//! its claims struct requires the handle and `exp` claims and would expose
//! the handle to this path; this decode target is `sub`-only by
//! construction.
//!
//! This module is deliberately scope-limited to the one claim. If a future
//! need for fuller JWT parsing arises, introduce a featureful module
//! separately — this one stays narrow so its PII surface stays reviewable.

use serde::Deserialize;
use tracing::debug;

/// Decode target for [`decode_jwt_sub`].
///
/// A single-field struct by design: no other claim (`email`, `name`,
/// `preferred_username`, the handle, …) can be returned, because no other
/// claim is ever deserialized. This is the structural PII guardrail the
/// unit tests pin.
#[derive(Debug, Deserialize)]
struct SubClaim {
    sub: String,
}

/// Extract the OIDC `sub` claim from a JWT, without signature verification.
///
/// Returns `Some(sub)` when the token's payload segment decodes to JSON
/// with a non-empty string `sub` claim; `None` on any failure (malformed
/// token, undecodable payload, missing / empty / non-string `sub`).
/// Failures are logged at `debug!` only — token presence is sensitive
/// metadata, so even a decode-failure log line must not surface at the
/// default `RUST_LOG` level.
///
/// Uses [`jsonwebtoken::dangerous::insecure_decode`] — the same decode
/// primitive `floxhub-client`'s `FloxhubToken` parsing uses — rather than
/// hand-rolling the base64url payload split. See the module docs for the
/// trust model.
pub fn decode_jwt_sub(token: &str) -> Option<String> {
    match jsonwebtoken::dangerous::insecure_decode::<SubClaim>(token) {
        Ok(data) if !data.claims.sub.is_empty() => Some(data.claims.sub),
        Ok(_) => {
            debug!("JWT decoded but 'sub' claim is empty; no auth subject");
            None
        },
        Err(err) => {
            debug!(error = %err, "could not decode JWT for 'sub' claim; no auth subject");
            None
        },
    }
}

/// Test-fixture support: build unsigned JWTs around arbitrary payloads.
///
/// `#[cfg(test)]`-gated but `pub(crate)` so sibling modules' tests (e.g.
/// the events wrapper) share one builder instead of copy-pasting the
/// encoding recipe.
#[cfg(test)]
pub(crate) mod test_helpers {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    /// Build a syntactically valid, unsigned JWT around an arbitrary JSON
    /// payload. The signature segment is garbage on purpose — decoding
    /// must never look at it.
    pub(crate) fn token_with_payload(payload: &serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"HS256"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string());
        format!("{header}.{body}.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA")
    }
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use serde_json::json;

    use super::test_helpers::token_with_payload;
    use super::*;

    #[test]
    fn decode_returns_sub_when_present() {
        let token = token_with_payload(&json!({"sub": "github|3670948", "exp": 9999999999u64}));
        assert_eq!(decode_jwt_sub(&token).as_deref(), Some("github|3670948"));
    }

    /// PII guardrail: a payload carrying every identifying claim the
    /// FloxHub token is known to include yields ONLY the `sub` value.
    /// Defensive against a future bug where the helper accidentally
    /// returns (or concatenates) the wrong claim: the result must not be
    /// an email (no `@`), the handle, or the display name.
    #[test]
    fn decode_returns_only_sub_never_email_or_handle() {
        let token = token_with_payload(&json!({
            "sub": "github|3670948",
            "email": "j@example.com",
            "name": "Jane",
            "preferred_username": "jane",
            "https://flox.dev/handle": "jane",
        }));
        let sub = decode_jwt_sub(&token).expect("sub decodes");
        assert_eq!(sub, "github|3670948");
        assert!(!sub.contains('@'), "must never be an email");
        assert_ne!(sub, "jane", "must never be the handle or username");
        assert_ne!(sub, "Jane", "must never be the display name");
    }

    #[test]
    fn decode_returns_none_when_sub_missing() {
        let token = token_with_payload(&json!({
            "email": "j@example.com",
            "name": "Jane",
            "preferred_username": "jane",
            "exp": 9999999999u64,
        }));
        assert_eq!(decode_jwt_sub(&token), None);
    }

    #[test]
    fn decode_returns_none_when_sub_is_empty() {
        let token = token_with_payload(&json!({"sub": ""}));
        assert_eq!(decode_jwt_sub(&token), None);
    }

    #[test]
    fn decode_returns_none_when_sub_is_non_string() {
        let token = token_with_payload(&json!({"sub": 12345}));
        assert_eq!(decode_jwt_sub(&token), None);
    }

    #[test]
    fn decode_returns_none_when_token_segments_missing() {
        assert_eq!(decode_jwt_sub(""), None);
        assert_eq!(decode_jwt_sub("not-a-jwt"), None);
        assert_eq!(decode_jwt_sub("only.one-dot"), None);
    }

    #[test]
    fn decode_returns_none_when_payload_b64_invalid() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"HS256"}"#);
        let token = format!("{header}.!!!not-base64url!!!.sig");
        assert_eq!(decode_jwt_sub(&token), None);
    }

    #[test]
    fn decode_returns_none_when_payload_not_json() {
        let header = URL_SAFE_NO_PAD.encode(r#"{"typ":"JWT","alg":"HS256"}"#);
        let body = URL_SAFE_NO_PAD.encode("this is not json");
        let token = format!("{header}.{body}.sig");
        assert_eq!(decode_jwt_sub(&token), None);
    }

    /// The `sub` of an expired token is still the correct attribution —
    /// expiry gates authentication, not identity. Pins that the decode
    /// applies no claim validation.
    #[test]
    fn decode_succeeds_for_expired_token() {
        let token = token_with_payload(&json!({
            "sub": "auth0|abcdef",
            // 2024-01-01T00:00:00Z — long expired.
            "exp": 1704063600u64,
        }));
        assert_eq!(decode_jwt_sub(&token).as_deref(), Some("auth0|abcdef"));
    }

    #[test]
    fn decode_passes_through_realistic_auth0_sub_shapes() {
        for sub in [
            "auth0|507f1f77bcf86cd799439011",
            "github|3670948",
            "google-oauth2|1078",
        ] {
            let token = token_with_payload(&json!({"sub": sub}));
            assert_eq!(decode_jwt_sub(&token).as_deref(), Some(sub));
        }
    }
}
