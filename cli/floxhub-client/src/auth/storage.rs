//! Integration between credential storage and runtime authentication.
//!
//! Ordinary callers use [AuthContext] accessors. Storage adapters explicitly
//! import [AuthContextStorageExt] to wire deferred loading and persistence.
//! Recorded facts are advisory until checked against the loaded credential.

use chrono::{DateTime, Utc};

use super::{AuthContext, CredentialKind};

/// Storage-only hooks for [AuthContext].
///
/// These methods require an explicit trait import; they are not part of the
/// ordinary authentication interface.
///
/// ```
/// use floxhub_client::AuthContext;
/// use floxhub_client::auth::storage::{AuthContextStorageExt, CachedFacts};
///
/// let context = AuthContext::deferred(CachedFacts::default(), AuthContext::default)
///     .with_identity_recorder(|_facts| {});
/// context.seed_from(&CachedFacts::default());
/// assert_eq!(context.cached_facts(), CachedFacts::default());
/// assert_eq!(context.handle(), None);
/// ```
///
/// Without the trait, neither construction nor instance hooks are available:
///
/// ```compile_fail,E0599
/// use floxhub_client::AuthContext;
/// use floxhub_client::auth::storage::CachedFacts;
///
/// let _ = AuthContext::deferred(CachedFacts::default(), AuthContext::default);
/// ```
///
/// ```compile_fail,E0599
/// use floxhub_client::AuthContext;
///
/// let _ = AuthContext::default().with_identity_recorder(|_| {});
/// ```
///
/// ```compile_fail,E0599
/// use floxhub_client::AuthContext;
///
/// let _ = AuthContext::default().cached_facts();
/// ```
///
/// ```compile_fail,E0599
/// use floxhub_client::AuthContext;
/// use floxhub_client::auth::storage::CachedFacts;
///
/// AuthContext::default().seed_from(&CachedFacts::default());
/// ```
pub trait AuthContextStorageExt: Sized {
    /// Retain startup facts while deferring the credential-store read.
    fn deferred(
        recorded: CachedFacts,
        resolve: impl Fn() -> AuthContext + Send + Sync + 'static,
    ) -> Self;

    /// Install the persistent cache writer used after successful identity lookups.
    fn with_identity_recorder(self, record: impl Fn(&CachedFacts) + Send + Sync + 'static) -> Self;

    /// Non-secret snapshot for persistence, without loading the credential.
    /// Ordinary callers should use accessors such as [AuthContext::handle].
    fn cached_facts(&self) -> CachedFacts;

    /// Seed an already-loaded credential from a fingerprint-checked record.
    /// Deferred credentials check their record when first loaded.
    fn seed_from(&self, recorded: &CachedFacts);
}

/// The non-secret facts about a credential, in the form an earlier invocation
/// can record and a later one read back without paying to load the credential
/// again.
///
/// Exposed by [`AuthContextStorageExt::cached_facts`] without loading a deferred
/// credential. Recorded facts are advisory until checked against that credential.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CachedFacts {
    /// A bearer credential is stored. Says nothing about whether the server
    /// will accept it; no local check can.
    pub logged_in: bool,
    /// Which kind of credential this is. Telemetry stamps it on every event,
    /// and reading the credential to learn it would defeat the deferral.
    pub kind: CredentialKind,
    /// Whether a FloxHub login gates this credential at all. False for an
    /// opaque token, whose validity only the server knows, and for Kerberos,
    /// which authenticates from the ccache and needs no FloxHub login.
    pub requires_login: bool,
    /// Local expiry, for the credentials that carry one.
    pub expires_at: Option<DateTime<Utc>>,
    /// The pseudonymous subject telemetry is attributed to.
    pub subject: Option<String>,
    /// The user's handle, when it is known — from a JWT's claims, a Kerberos
    /// principal, or a `/me` resolution that has already happened.
    pub handle: Option<String>,
    /// Which credential these facts describe, as [`credential_fingerprint`].
    ///
    /// The facts are keyed by FloxHub instance, not by credential — the whole
    /// point is to answer before the credential has been loaded — so this is
    /// how a caller that *does* hold the secret checks that the record is
    /// about the credential in its hand. See [`AuthContextStorageExt::seed_from`].
    pub fingerprint: Option<String>,
}

/// A stable, non-reversible name for a credential.
///
/// Recorded beside the facts so a caller holding the secret can tell whether
/// they describe it. blake3 over a high-entropy bearer token is not
/// invertible, and the record is written `0600` regardless.
pub fn credential_fingerprint(secret: &str) -> String {
    blake3::hash(secret.as_bytes()).to_hex().to_string()
}

impl CachedFacts {
    /// Whether the recorded credential needs a FloxHub login.
    ///
    /// `requires_login` is what keeps this honest for the credentials that
    /// carry no bearer secret: Kerberos records `logged_in: false`, and
    /// without the gate that would read as "not logged in" for a mode where
    /// logging in is not a thing.
    pub fn is_unauthenticated(&self) -> bool {
        self.requires_login
            && (!self.logged_in || self.expires_at.is_some_and(|expiry| expiry <= Utc::now()))
    }
}
