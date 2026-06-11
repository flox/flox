//! Narrowly-scoped JWT helper — extracts the OIDC `sub` claim from a
//! JWT payload **without verifying the signature**.
//!
//! Used by [`crate::utils::events::build_events_client`] to populate the
//! canonical envelope's `auth_subject` field from the FloxHub token, so
//! per-invocation events ship with a stable per-account identifier when
//! one is available.
//!
//! ## Trust model
//!
//! The CLI is reading back a token it received from the FloxHub auth
//! service and stamping a claim onto its own outgoing events. It is **not**
//! authenticating to a downstream system with the extracted value, so
//! signature verification is unnecessary — a tampered token would only
//! tamper with the user's own attribution and would not gain access to
//! anything. This mirrors the trust model in
//! [`flox_catalog::token::FloxhubToken::from_str`], which uses the same
//! `jsonwebtoken::dangerous::insecure_decode` path against the same
//! token. The deliberate divergence is that this helper does **not**
//! require `exp`, so an expired token's `sub` still produces an
//! attribution — `sub` is stable across the account's lifetime, and
//! token refresh / re-auth happens elsewhere.
//!
//! ## What we extract
//!
//! Only the `sub` claim. The decoder is structurally incapable of
//! returning `email`, `name`, `preferred_username`, or any other claim
//! — the deserialization target is a struct with exactly one field.
//! This is defence-in-depth against a future change that accidentally
//! widens the field set; unit tests assert the surface stays narrow.
//!
//! ## What `sub` actually identifies
//!
//! For tokens issued from social-login connections, `sub` takes the
//! form `<connection>|<provider-id>` (e.g. `github|<numeric-id>`,
//! `google-oauth2|<numeric-id>`). The numeric portion of a
//! `github|<id>` value resolves via the public, unauthenticated GitHub
//! API to a login, display name, and avatar — it is **not** opaque,
//! and downstream storage / access policy should treat this column at
//! the same retention and access tier as a public account handle.
//! For Auth0-native connections the value is provider-opaque.

use serde::Deserialize;
use tracing::debug;

/// The only claim we extract. Deliberately a single-field struct so a
/// future change can't accidentally widen the surface. Intentionally
/// NOT `Debug` — `sub` is sensitive and a stray `debug!(claims =
/// ?data.claims, ...)` should not silently surface it through a
/// tracing subscriber.
#[derive(Deserialize)]
struct SubOnlyClaims {
    sub: Option<String>,
}

/// Extract the `sub` claim from a JWT payload without verifying the
/// signature. Returns `None` on any failure — malformed token, missing
/// `sub`, empty `sub`, or any decode error. Decoding errors are logged
/// at `debug!` only — token presence is sensitive metadata and even an
/// error log should not surface at default `RUST_LOG`.
///
/// See the module rustdoc for the trust model behind skipping signature
/// verification.
pub(crate) fn decode_jwt_sub(token: &str) -> Option<String> {
    match jsonwebtoken::dangerous::insecure_decode::<SubOnlyClaims>(token) {
        Ok(data) => data.claims.sub.filter(|s| !s.is_empty()),
        Err(err) => {
            debug!(error = %err, "auth_subject: JWT decode failed");
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use jsonwebtoken::{EncodingKey, Header, encode};
    use serde::Serialize;
    use serde_json::json;

    use super::*;

    /// Build a synthetic JWT — HS256 signed with a dummy secret. The
    /// signature is irrelevant because `decode_jwt_sub` uses
    /// `insecure_decode`, which skips signature verification.
    fn synthetic_jwt<T: Serialize>(payload: &T) -> String {
        encode(
            &Header::default(),
            payload,
            &EncodingKey::from_secret(b"test"),
        )
        .expect("encode synthetic JWT")
    }

    /// Happy path: a well-formed payload with a non-empty `sub` returns
    /// `Some(sub)`.
    #[test]
    fn decode_returns_sub_when_present() {
        let token = synthetic_jwt(&json!({ "sub": "github|99999999999" }));
        assert_eq!(
            decode_jwt_sub(&token),
            Some("github|99999999999".to_string())
        );
    }

    /// PII guardrail: a payload with `sub` AND `email`/`name`/
    /// `preferred_username` returns ONLY the `sub`. The struct-level
    /// `SubOnlyClaims` deserialization makes this structurally
    /// impossible; the test pins the contract so a future change
    /// widening the struct fails CI.
    #[test]
    fn decode_returns_only_sub_never_email_or_handle() {
        let token = synthetic_jwt(&json!({
            "sub": "github|99999999999",
            "email": "alice@example.com",
            "name": "Alice Example",
            "preferred_username": "alice",
            "https://flox.dev/handle": "alice",
        }));
        let out = decode_jwt_sub(&token).expect("sub present");
        assert_eq!(out, "github|99999999999");
        // Defence-in-depth: a future bug that returned the wrong claim
        // would fail at least one of these.
        assert!(!out.contains('@'), "email leaked: {out:?}");
        assert!(!out.contains(' '), "name leaked: {out:?}");
        assert_ne!(out, "alice", "handle leaked");
        assert_ne!(out, "Alice Example", "name leaked");
    }

    /// Missing `sub` claim: returns None.
    #[test]
    fn decode_returns_none_when_sub_missing() {
        let token = synthetic_jwt(&json!({
            "email": "alice@example.com",
            "name": "Alice",
        }));
        assert_eq!(decode_jwt_sub(&token), None);
    }

    /// Empty `sub` claim: returns None (an empty string is not a
    /// usable identifier).
    #[test]
    fn decode_returns_none_when_sub_empty() {
        let token = synthetic_jwt(&json!({ "sub": "" }));
        assert_eq!(decode_jwt_sub(&token), None);
    }

    /// Non-string `sub` claim (integer, null, object, array, bool).
    /// `Option<String>` accepts `null` as `None` (so the helper
    /// returns `None`); the other variants fail the deserialization
    /// of the whole claim set and also surface as `None`.
    #[test]
    fn decode_returns_none_when_sub_shape_unexpected() {
        for payload in [
            json!({ "sub": 12345 }),
            json!({ "sub": null }),
            json!({ "sub": { "nested": "value" } }),
            json!({ "sub": ["array"] }),
            json!({ "sub": true }),
        ] {
            let token = synthetic_jwt(&payload);
            assert_eq!(
                decode_jwt_sub(&token),
                None,
                "expected None for payload {payload}"
            );
        }
    }

    /// Payload is valid JSON but not a JSON object (string, array,
    /// number, bool, null): `SubOnlyClaims` is a struct, so
    /// deserialization fails and the helper returns None.
    #[test]
    fn decode_returns_none_when_payload_not_a_json_object() {
        for payload in [
            json!("hello"),
            json!([1, 2, 3]),
            json!(42),
            json!(true),
            json!(null),
        ] {
            let token = synthetic_jwt(&payload);
            assert_eq!(
                decode_jwt_sub(&token),
                None,
                "expected None for payload {payload}"
            );
        }
    }

    /// Malformed token shape — wrong segment count, invalid base64,
    /// or empty string. All return None silently.
    #[test]
    fn decode_returns_none_when_token_malformed() {
        for token in ["", "notajwt", "a.b", "header.!!!not_base64!!!.signature"] {
            assert_eq!(decode_jwt_sub(token), None, "expected None for {token:?}");
        }
    }

    /// Realistic Auth0-shape `sub` values pass through verbatim. The
    /// pipe character is the Auth0 connection separator; the value
    /// after it is the connection-specific user id (stable across the
    /// account's lifetime).
    #[test]
    fn decode_passes_through_realistic_auth0_sub_shapes() {
        for sub in [
            "auth0|abcdef1234567890",
            "github|99999999999",
            "google-oauth2|108112233445566778899",
        ] {
            let token = synthetic_jwt(&json!({ "sub": sub }));
            assert_eq!(decode_jwt_sub(&token).as_deref(), Some(sub));
        }
    }
}
