//! Credential material and the facts it can answer locally.

use crate::auth::kerberos::KerberosMaterial;
use crate::auth::storage::{CachedFacts, credential_fingerprint};
use crate::auth::token::{
    ACCESS_TOKEN_PREFIX,
    AccessToken,
    BareToken,
    FloxhubToken,
    PERSONAL_ACCESS_TOKEN_PREFIX,
    SERVICE_ACCOUNT_TOKEN_PREFIX,
};

/// Which kind of credential is in play, without the material.
///
/// This is the credential kind with the secret stripped out — the one
/// part of the variant a record can carry, and the only part a caller needs
/// when it wants to describe the credential rather than use it. Telemetry is
/// the caller today: it reports the credential kind on every event, and
/// deriving that from the variant would mean reading the credential on every
/// invocation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    /// No credential is stored.
    #[default]
    NotLoggedIn,
    /// An Auth0-shaped JWT, identity in its claims.
    Auth0,
    /// A JWT without the handle claim.
    Bare,
    /// An opaque `flox_pat_` personal access token.
    PersonalAccessToken,
    /// An opaque `flox_sat_` service account token.
    ServiceAccountToken,
    /// An opaque token matching no known prefix — an issuer may mint one.
    OpaqueToken,
    /// Kerberos, which uses no FloxHub token at all.
    Kerberos,
}

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

/// Credential material resolved by `AuthContext`.
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
/// Callers needing a specific token format can inspect this through
/// `AuthContext::credential`.
#[derive(Clone)]
pub enum Credential {
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

impl Credential {
    /// Return the user's handle, when it is known locally: JWT claims, a
    /// Kerberos principal, or a token whose identity was already resolved
    /// and cached. Never touches the network.
    pub fn handle(&self) -> Option<String> {
        match self {
            Credential::Auth0(Some(token)) => Some(token.handle().to_string()),
            Credential::Auth0(None) => None,
            Credential::Bare(token) => token.handle(),
            Credential::AccessToken(token) => token.handle(),
            Credential::Kerberos(Some(material)) => Some(material.principal.clone()),
            Credential::Kerberos(None) => None,
        }
    }

    /// Return the pseudonymous subject identifier for telemetry attribution,
    /// if one is available locally or in the process-wide identity cache.
    ///
    /// Auth0 tokens carry the OIDC `sub` claim ([`FloxhubToken::sub`]) —
    /// opaque and stable across the user's lifetime, so it remains valid
    /// attribution even when the token has expired. PATs and SATs read the
    /// `/me.user_id` value after identity resolution has populated the cache.
    /// Kerberos has no pseudonymous equivalent today (the principal is
    /// directly identifying), so kerberos-mode invocations return `None`.
    ///
    /// [`FloxhubToken::sub`]: crate::auth::token::FloxhubToken::sub
    pub fn user_subject(&self) -> Option<String> {
        match self {
            Credential::Auth0(Some(token)) => token.sub().map(str::to_owned),
            Credential::Auth0(None) => None,
            Credential::Bare(token) => token.sub().map(str::to_owned),
            Credential::AccessToken(token) => {
                crate::auth::identity::cached_identity(token.secret())
                    .and_then(|identity| identity.sub)
            },
            Credential::Kerberos(_) => None,
        }
    }

    /// Return the raw token secret, if this credential carries one.
    ///
    /// Kerberos does not use bearer tokens, so it has no secret.
    pub fn token_secret(&self) -> Option<&str> {
        match self {
            Credential::Auth0(Some(token)) => Some(token.secret()),
            Credential::Auth0(None) => None,
            Credential::Bare(token) => Some(token.secret()),
            Credential::AccessToken(token) => Some(token.secret()),
            Credential::Kerberos(_) => None,
        }
    }

    /// Create an [`Credential`] from a stored token, routing by what the
    /// credential's claims answer locally:
    ///
    /// - `flox_`-prefixed token: [`Credential::AccessToken`] — opaque by
    ///   fiat, never decoded.
    /// - Auth0-shaped JWT (handle claim and expiry): [`Credential::Auth0`].
    /// - Any other decodable JWT: [`Credential::Bare`].
    /// - Anything else: [`Credential::AccessToken`] — an issuer may mint
    ///   opaque access tokens.
    /// - No token: `Auth0(None)` (not logged in).
    ///
    /// Routing is total: no local check can reject a token, and the
    /// server's 401 is the authority on validity.
    pub fn new_from_token(token: Option<&str>) -> Self {
        let Some(token) = token else {
            return Credential::Auth0(None);
        };
        if token.starts_with(ACCESS_TOKEN_PREFIX) {
            return Credential::AccessToken(AccessToken::new(token.to_string()));
        }
        match FloxhubToken::new(token.to_string()) {
            Ok(parsed) => Credential::Auth0(Some(parsed)),
            Err(_) => match BareToken::new(token.to_string()) {
                Ok(parsed) => Credential::Bare(parsed),
                Err(_) => Credential::AccessToken(AccessToken::new(token.to_string())),
            },
        }
    }

    /// Create a Kerberos [`Credential`]: resolves the principal and embeds
    /// a SPNEGO token generator; returns `Kerberos(None)` (with a warning
    /// log) if the ticket cannot be resolved. FloxHub tokens are not used.
    pub fn new_kerberos() -> Self {
        crate::auth::kerberos::kerberos_credential()
    }

    /// Which kind of credential this is, without its material.
    ///
    /// The opaque arms read the token's prefix, which is the only thing about
    /// an opaque token that is ever parsed — it names the kind and nothing
    /// else. Authentication still treats them uniformly.
    pub fn kind(&self) -> CredentialKind {
        match self {
            Credential::Auth0(Some(_)) => CredentialKind::Auth0,
            Credential::Auth0(None) => CredentialKind::NotLoggedIn,
            Credential::Bare(_) => CredentialKind::Bare,
            Credential::AccessToken(token) => {
                if token.secret().starts_with(PERSONAL_ACCESS_TOKEN_PREFIX) {
                    CredentialKind::PersonalAccessToken
                } else if token.secret().starts_with(SERVICE_ACCOUNT_TOKEN_PREFIX) {
                    CredentialKind::ServiceAccountToken
                } else {
                    CredentialKind::OpaqueToken
                }
            },
            Credential::Kerberos(_) => CredentialKind::Kerberos,
        }
    }

    /// The non-secret facts worth recording about this credential.
    ///
    /// Derived per variant, because what a credential answers locally differs
    /// by kind: an Auth0 JWT carries expiry and subject in its claims, a bare
    /// JWT carries them only when the issuer emitted them, an opaque token
    /// carries neither until `/me` has resolved it, and Kerberos is not
    /// login-gated at all.
    pub fn cached_facts(&self) -> CachedFacts {
        CachedFacts {
            logged_in: self.token_secret().is_some(),
            kind: self.kind(),
            requires_login: matches!(self, Credential::Auth0(_) | Credential::Bare(_)),
            expires_at: match self {
                Credential::Auth0(Some(token)) => Some(token.expires_at()),
                Credential::Bare(token) => token.expires_at(),
                // An opaque token's expiry is whatever `/me` reported, so it
                // is known exactly when its identity is. This never reaches
                // `is_unauthenticated`, which `requires_login` already
                // answers `false` for — the server remains the authority on
                // an opaque token's validity.
                Credential::AccessToken(token) => token.expires_at(),
                Credential::Auth0(None) | Credential::Kerberos(_) => None,
            },
            subject: self.user_subject(),
            handle: self.handle(),
            fingerprint: self.token_secret().map(credential_fingerprint),
        }
    }

    /// Adopt a recorded identity as this credential's, sparing the `/me`
    /// round trip that would otherwise resolve it.
    ///
    /// Only the credentials whose identity costs a request have anything to
    /// gain: an Auth0 JWT reads its handle from its own claims and Kerberos
    /// from its principal, so both ignore the record. And only when the
    /// record describes *this* credential — the fingerprint is checked
    /// against the secret in hand, so a record left by another account, or by
    /// another `flox` writing the keyring behind our back, is ignored rather
    /// than believed.
    pub fn seed_from(&self, recorded: &CachedFacts) {
        if !matches!(self, Credential::Bare(_) | Credential::AccessToken(_)) {
            return;
        }
        let (Some(secret), Some(handle)) = (self.token_secret(), recorded.handle.clone()) else {
            return;
        };
        if recorded.fingerprint.as_deref() != Some(&credential_fingerprint(secret)) {
            return;
        }
        crate::auth::identity::cache_identity(secret, &crate::auth::UserIdentity {
            handle,
            expires_at: recorded.expires_at,
            sub: recorded.subject.clone(),
        });
    }
}

impl std::fmt::Debug for Credential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Credential::Auth0(Some(_)) => f.debug_tuple("Auth0").field(&"<token>").finish(),
            Credential::Auth0(None) => f.write_str("Auth0(None)"),
            Credential::Bare(token) => f.debug_tuple("Bare").field(&token).finish(),
            Credential::AccessToken(token) => f.debug_tuple("AccessToken").field(&token).finish(),
            Credential::Kerberos(Some(material)) => f
                .debug_struct("Kerberos")
                .field("principal", &material.principal)
                .finish_non_exhaustive(),
            Credential::Kerberos(None) => f.write_str("Kerberos(None)"),
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
            Credential::Auth0(Some(token)).user_subject().as_deref(),
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
            Credential::Auth0(Some(token)).user_subject().as_deref(),
            Some("github|424242")
        );
    }

    #[test]
    fn user_subject_is_none_without_sub_token_or_auth0() {
        let token = FloxhubToken::from_str(FAKE_TOKEN).expect("token parses");
        assert_eq!(Credential::Auth0(Some(token)).user_subject(), None);
        assert_eq!(Credential::Auth0(None).user_subject(), None);
        assert_eq!(Credential::Kerberos(None).user_subject(), None);
    }

    #[test]
    fn is_unauthenticated_covers_missing_and_expired_auth0_tokens() {
        let valid = FloxhubToken::from_str(FAKE_TOKEN).expect("token parses");
        let expired = FloxhubToken::from_str(FAKE_EXPIRED_TOKEN_WITH_SUB).expect("token parses");
        assert!(expired.is_expired(), "test premise: token is expired");

        assert!(Credential::Auth0(None).cached_facts().is_unauthenticated());
        assert!(
            Credential::Auth0(Some(expired))
                .cached_facts()
                .is_unauthenticated()
        );
        assert!(
            !Credential::Auth0(Some(valid))
                .cached_facts()
                .is_unauthenticated()
        );
        assert!(!pat_unresolved().cached_facts().is_unauthenticated());
        assert!(
            !Credential::Kerberos(None)
                .cached_facts()
                .is_unauthenticated()
        );
    }

    fn pat_unresolved() -> Credential {
        Credential::AccessToken(AccessToken::new("flox_pat_secret".to_string()))
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
        let auth = Credential::AccessToken(token);

        assert_eq!(auth.handle(), Some("testuser".to_string()));
    }

    #[test]
    fn pat_subject_reads_the_cached_identity() {
        let token = AccessToken::new("flox_pat_context-subject-test".to_string());
        crate::auth::identity::cache_identity(token.secret(), &crate::auth::UserIdentity {
            handle: "testuser".to_string(),
            sub: Some("auth0|123".to_string()),
            expires_at: None,
        });
        let auth = Credential::AccessToken(token);

        assert_eq!(auth.user_subject().as_deref(), Some("auth0|123"));
    }

    #[test]
    fn pat_debug_redacts_the_secret() {
        let auth = pat_unresolved();
        assert!(!format!("{auth:?}").contains("flox_pat_secret"));
    }

    /// The record's whole purpose for an opaque token: the handle `/me`
    /// returned last time, adopted without a request.
    #[test]
    fn seed_from_adopts_a_handle_recorded_for_this_credential() {
        let secret = "flox_pat_seed-adopts";
        let auth = Credential::new_from_token(Some(secret));
        assert_eq!(auth.handle(), None, "unresolved to begin with");

        auth.seed_from(&CachedFacts {
            handle: Some("seeded-user".to_string()),
            subject: Some("auth0|seeded".to_string()),
            fingerprint: Some(credential_fingerprint(secret)),
            ..CachedFacts::default()
        });

        assert_eq!(auth.handle(), Some("seeded-user".to_string()));
        assert_eq!(auth.user_subject(), Some("auth0|seeded".to_string()));
    }

    /// The fingerprint is what makes a recorded handle safe to believe: a
    /// record left by another account must not name this credential's user.
    #[test]
    fn seed_from_ignores_a_record_for_another_credential() {
        let auth = Credential::new_from_token(Some("flox_pat_seed-wrong-fingerprint"));

        auth.seed_from(&CachedFacts {
            handle: Some("someone-else".to_string()),
            fingerprint: Some(credential_fingerprint("a-different-secret")),
            ..CachedFacts::default()
        });

        assert_eq!(auth.handle(), None);
    }

    /// A record with no fingerprint cannot be attributed to any credential,
    /// so it is never adopted.
    #[test]
    fn seed_from_ignores_a_record_without_a_fingerprint() {
        let auth = Credential::new_from_token(Some("flox_pat_seed-no-fingerprint"));

        auth.seed_from(&CachedFacts {
            handle: Some("someone-else".to_string()),
            fingerprint: None,
            ..CachedFacts::default()
        });

        assert_eq!(auth.handle(), None);
    }

    /// A JWT answers its own handle from its claims, so a record — even a
    /// correctly fingerprinted one — has nothing to add and must not
    /// override it.
    #[test]
    fn seed_from_leaves_a_jwt_answering_from_its_claims() {
        let auth = Credential::new_from_token(Some(FAKE_TOKEN));
        let secret = auth.token_secret().expect("a JWT carries a secret");

        auth.seed_from(&CachedFacts {
            handle: Some("not-this-one".to_string()),
            fingerprint: Some(credential_fingerprint(secret)),
            ..CachedFacts::default()
        });

        assert_eq!(auth.handle(), Some("test".to_string()));
    }

    /// The facts a credential records are what a later invocation reads back,
    /// so an opaque token contributes everything `/me` resolved for it.
    #[test]
    fn cached_facts_of_a_resolved_opaque_token() {
        let secret = "flox_pat_cached-facts-test";
        let auth = Credential::new_from_token(Some(secret));
        crate::auth::identity::cache_identity(secret, &crate::auth::UserIdentity {
            handle: "factsuser".to_string(),
            sub: Some("auth0|facts".to_string()),
            expires_at: None,
        });

        assert_eq!(auth.cached_facts(), CachedFacts {
            logged_in: true,
            kind: CredentialKind::PersonalAccessToken,
            // Opaque: only the server can judge it, so no local gate.
            requires_login: false,
            expires_at: None,
            subject: Some("auth0|facts".to_string()),
            handle: Some("factsuser".to_string()),
            fingerprint: Some(credential_fingerprint(secret)),
        });
    }

    #[test]
    fn jwt_handle_derives_from_claims() {
        let auth = Credential::Auth0(Some(FAKE_TOKEN.parse().unwrap()));
        assert_eq!(auth.handle(), Some("test".to_string()));
    }

    #[test]
    fn new_from_token_routes_flox_prefix_to_access_token() {
        // Any flox_-prefixed token is an opaque access token, including
        // personal and service account tokens.
        for secret in ["flox_pat_abc123", "flox_sat_abc123"] {
            let auth = Credential::new_from_token(Some(secret));
            let Credential::AccessToken(token) = auth else {
                panic!("expected AccessToken, got {auth:?}");
            };
            assert_eq!(token.secret(), secret);
        }
    }

    #[test]
    fn new_from_token_routes_jwt_to_auth0() {
        let auth = Credential::new_from_token(Some(FAKE_TOKEN));
        let Credential::Auth0(Some(token)) = auth else {
            panic!("expected Auth0, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN);
    }

    #[test]
    fn new_from_token_without_token_is_not_logged_in() {
        let auth = Credential::new_from_token(None);
        assert!(matches!(auth, Credential::Auth0(None)));
    }

    #[test]
    fn new_from_token_routes_claimless_jwt_to_bare() {
        let auth = Credential::new_from_token(Some(FAKE_TOKEN_NO_HANDLE));
        let Credential::Bare(token) = auth else {
            panic!("expected Bare, got {auth:?}");
        };
        assert_eq!(token.secret(), FAKE_TOKEN_NO_HANDLE);
    }

    #[test]
    fn new_from_token_carries_a_non_jwt_opaquely() {
        // No local check can reject a token — an issuer may mint opaque
        // access tokens, so a non-decodable string is a credential whose
        // validity only the server can judge.
        let auth = Credential::new_from_token(Some("not-a-jwt"));
        let Credential::AccessToken(token) = auth else {
            panic!("expected AccessToken, got {auth:?}");
        };
        assert_eq!(token.secret(), "not-a-jwt");
    }

    #[test]
    fn bare_token_subject_and_cached_handle() {
        // A bare token contributes its sub for telemetry, and its handle
        // comes from the /me-filled cache, like an opaque token's.
        let token = test_bare_token("context-bare-handle-test");
        let auth = Credential::Bare(token.clone());
        assert_eq!(
            auth.user_subject().as_deref(),
            Some("context-bare-handle-test")
        );
        assert_eq!(auth.handle(), None);

        crate::auth::identity::cache_identity(token.secret(), &test_identity("dexter"));
        assert_eq!(Credential::Bare(token).handle(), Some("dexter".to_string()));
    }
}
