//! Authentication with credential loading and identity caching handled internally.

use std::fmt;
use std::sync::{Arc, OnceLock};

use url::Url;

use super::credential::Credential;
use super::storage::{AuthContextStorageExt, CachedFacts};
use super::{
    AccessToken,
    AuthFailure,
    AuthHeaderError,
    BareToken,
    CredentialKind,
    FloxhubToken,
    KerberosMaterial,
    UserIdentity,
    identity,
};
use crate::FloxhubClient;
use crate::accounts::MeError;

type IdentityRecorder = Arc<dyn Fn(&CachedFacts) + Send + Sync>;

enum CredentialSource {
    Resolved(Credential),
    Deferred {
        recorded: CachedFacts,
        resolved: OnceLock<Credential>,
        resolve: Box<dyn Fn() -> AuthContext + Send + Sync>,
    },
}

/// Authentication for one invocation.
///
/// Startup checks use recorded non-secret facts. Operations that need a secret
/// or an identity load the credential automatically, at most once across clones.
/// Identity lookups reuse a fingerprint-checked cache and record successful
/// resolutions for later invocations.
#[derive(Clone)]
pub struct AuthContext {
    source: Arc<CredentialSource>,
    record_identity: Option<IdentityRecorder>,
}

impl AuthContext {
    fn from_credential(credential: Credential) -> Self {
        Self {
            source: Arc::new(CredentialSource::Resolved(credential)),
            record_identity: None,
        }
    }

    /// Parse a token's locally available claims without contacting FloxHub.
    pub fn new_from_token(token: Option<&str>) -> Self {
        Self::from_credential(Credential::new_from_token(token))
    }

    pub fn from_auth0_token(token: Option<FloxhubToken>) -> Self {
        Self::from_credential(Credential::Auth0(token))
    }

    pub fn from_bare_token(token: BareToken) -> Self {
        Self::from_credential(Credential::Bare(token))
    }

    pub fn from_access_token(token: AccessToken) -> Self {
        Self::from_credential(Credential::AccessToken(token))
    }

    pub fn from_kerberos(material: Option<KerberosMaterial>) -> Self {
        Self::from_credential(Credential::Kerberos(material))
    }

    /// Acquire the Kerberos credential only when an operation needs it.
    pub fn new_kerberos() -> Self {
        Self::deferred(Credential::Kerberos(None).cached_facts(), || {
            Self::from_credential(Credential::new_kerberos())
        })
    }

    fn credential(&self) -> &Credential {
        match self.source.as_ref() {
            CredentialSource::Resolved(credential) => credential,
            CredentialSource::Deferred {
                recorded,
                resolved,
                resolve,
            } => resolved.get_or_init(|| {
                let credential = resolve().credential().clone();
                credential.seed_from(recorded);
                credential
            }),
        }
    }

    pub fn is_unauthenticated(&self) -> bool {
        self.cached_facts().is_unauthenticated()
    }

    pub fn user_subject(&self) -> Option<String> {
        self.cached_facts().subject
    }

    pub fn kind(&self) -> CredentialKind {
        self.cached_facts().kind
    }

    /// Return the bearer secret, loading the credential if necessary.
    pub fn token_secret(&self) -> Option<&str> {
        self.credential().token_secret()
    }

    /// Return the Auth0 token for operations specific to that token format.
    pub fn auth0_token(&self) -> Option<&FloxhubToken> {
        match self.credential() {
            Credential::Auth0(token) => token.as_ref(),
            _ => None,
        }
    }

    /// Return the Kerberos principal, loading the credential if necessary.
    ///
    /// This checks local ticket availability without generating an HTTP auth
    /// token. `None` means no Kerberos credential is available.
    pub fn kerberos_principal(&self) -> Option<&str> {
        match self.credential() {
            Credential::Kerberos(Some(material)) => Some(&material.principal),
            _ => None,
        }
    }

    /// Produce authentication for an outgoing request.
    pub fn authorization_header(&self, url: &Url) -> Option<Result<String, AuthHeaderError>> {
        self.credential().authorization_header(url)
    }

    /// Return the currently known handle without loading credentials or doing I/O.
    ///
    /// `None` means the handle is unknown, not necessarily that the user is
    /// unauthenticated. An environment or manually configured token may never
    /// have had its identity resolved; its saved record may be missing, corrupt,
    /// unwritable, or describe a different token. Credentials from before
    /// identity caching, an unloaded Kerberos principal, and missing credentials
    /// can also leave the handle unknown.
    ///
    /// Before the credential is loaded, a recorded handle is advisory and may
    /// be stale. Use [`Self::identity`] when the operation requires an identity
    /// checked against the current credential, rather than an optional hint.
    pub fn handle(&self) -> Option<String> {
        self.cached_facts().handle
    }

    /// Resolve the identity, including expiry and the pseudonymous subject.
    ///
    /// `Ok(None)` means the identity could not be verified because FloxHub was
    /// unavailable. Missing credentials and server rejection are errors. Local
    /// expiry is returned in the identity so callers decide whether it blocks
    /// their operation.
    ///
    /// Successful network lookups update both the process cache and the
    /// persistent record. JWT identity claims and Kerberos principals answer
    /// locally.
    pub async fn identity(
        &self,
        client: &FloxhubClient,
    ) -> Result<Option<UserIdentity>, AuthFailure> {
        self.resolve_identity(client, false).await
    }

    /// Fetch identity from `/me` again and update the caches on success.
    ///
    /// Auth status uses this to detect revoked tokens and renamed handles.
    /// This does not reload or renew the credential. JWT identity claims and
    /// Kerberos principals still answer locally.
    pub async fn refresh_identity(
        &self,
        client: &FloxhubClient,
    ) -> Result<Option<UserIdentity>, AuthFailure> {
        self.resolve_identity(client, true).await
    }

    async fn resolve_identity(
        &self,
        client: &FloxhubClient,
        refresh: bool,
    ) -> Result<Option<UserIdentity>, AuthFailure> {
        let credential = self.credential();
        let identity = match credential {
            Credential::Auth0(Some(token)) => UserIdentity {
                handle: token.handle().to_string(),
                sub: token.sub().map(str::to_owned),
                expires_at: Some(token.expires_at()),
            },
            Credential::Auth0(None) => return Err(AuthFailure::NotLoggedIn),
            Credential::Kerberos(Some(material)) => UserIdentity {
                handle: material.principal.clone(),
                sub: None,
                expires_at: None,
            },
            Credential::Kerberos(None) => return Err(AuthFailure::NoKerberosTicket),
            Credential::Bare(_) | Credential::AccessToken(_) => {
                let secret = credential
                    .token_secret()
                    .expect("bearer credential has a token");
                let cached = (!refresh)
                    .then(|| identity::cached_identity(secret))
                    .flatten();
                let mut resolved = match cached {
                    Some(identity) => identity,
                    None => match client.accounts().me(secret).await {
                        Ok(resolved) => {
                            identity::cache_identity(secret, &resolved);
                            resolved
                        },
                        Err(MeError::Unauthorized) => return Err(AuthFailure::TokenExpired),
                        Err(err) => {
                            tracing::debug!(error = %err, "could not resolve identity");
                            return Ok(None);
                        },
                    },
                };
                // JWT expiry describes the presenting credential; /me may
                // describe a stored access token instead.
                if let Credential::Bare(token) = credential {
                    resolved.expires_at = token.expires_at().or(resolved.expires_at);
                }
                resolved
            },
        };
        if let Some(record) = &self.record_identity {
            record(&self.cached_facts());
        }
        Ok(Some(identity))
    }
}

impl AuthContextStorageExt for AuthContext {
    fn deferred(
        recorded: CachedFacts,
        resolve: impl Fn() -> AuthContext + Send + Sync + 'static,
    ) -> Self {
        Self {
            source: Arc::new(CredentialSource::Deferred {
                recorded,
                resolved: OnceLock::new(),
                resolve: Box::new(resolve),
            }),
            record_identity: None,
        }
    }

    fn with_identity_recorder(
        mut self,
        record: impl Fn(&CachedFacts) + Send + Sync + 'static,
    ) -> Self {
        self.record_identity = Some(Arc::new(record));
        self
    }

    fn cached_facts(&self) -> CachedFacts {
        match self.source.as_ref() {
            CredentialSource::Resolved(credential) => credential.cached_facts(),
            CredentialSource::Deferred {
                recorded, resolved, ..
            } => resolved
                .get()
                .map(Credential::cached_facts)
                .unwrap_or_else(|| recorded.clone()),
        }
    }

    fn seed_from(&self, recorded: &CachedFacts) {
        match self.source.as_ref() {
            CredentialSource::Resolved(credential) => credential.seed_from(recorded),
            CredentialSource::Deferred { resolved, .. } => {
                if let Some(credential) = resolved.get() {
                    credential.seed_from(recorded);
                }
            },
        }
    }
}

impl Default for AuthContext {
    fn default() -> Self {
        Self::from_auth0_token(None)
    }
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthContext")
            .field("facts", &self.cached_facts())
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use httpmock::MockServer;
    use serde_json::json;

    use super::*;
    use crate::auth::token::test_helpers::test_bare_token;
    use crate::client::test_helpers::client_config;

    /// The record an invocation writes when nobody is logged in: a FloxHub
    /// login is what would fix it, so `requires_login` is set.
    fn logged_out() -> CachedFacts {
        AuthContext::from_auth0_token(None).cached_facts()
    }

    #[test]
    fn deferred_context_is_not_resolved_until_it_is_needed() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let lazy = AuthContext::deferred(logged_out(), move || {
            counter.fetch_add(1, Ordering::SeqCst);
            AuthContext::from_auth0_token(None)
        });

        assert!(!lazy.cached_facts().logged_in);
        assert!(lazy.is_unauthenticated());
        assert_eq!(lazy.user_subject(), None);
        assert_eq!(calls.load(Ordering::SeqCst), 0);

        lazy.token_secret();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Every clone shares one resolution: the handle is threaded through the
    /// client, the SDK and the transport adapters, and a read per hop would
    /// defeat the point.
    #[test]
    fn clones_share_a_single_resolution() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&calls);
        let lazy = AuthContext::deferred(logged_out(), move || {
            counter.fetch_add(1, Ordering::SeqCst);
            AuthContext::from_bare_token(test_bare_token("lazy-clone-test"))
        });

        lazy.clone().token_secret();
        lazy.clone().token_secret();
        lazy.token_secret();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// A recorded summary that disagrees with the stored credential is
    /// superseded once the credential is actually read.
    #[test]
    fn resolving_supersedes_stale_recorded_properties() {
        let lazy = AuthContext::deferred(logged_out(), || {
            AuthContext::from_bare_token(test_bare_token("lazy-stale-test"))
        });

        assert!(lazy.is_unauthenticated());
        lazy.token_secret();
        assert!(!lazy.is_unauthenticated());
    }

    #[test]
    fn deferred_context_exposes_recorded_properties_without_resolving() {
        let recorded = CachedFacts {
            logged_in: true,
            kind: CredentialKind::Auth0,
            requires_login: true,
            expires_at: None,
            subject: Some("cached-subject".to_string()),
            handle: Some("cached-user".to_string()),
            fingerprint: Some("cached-fingerprint".to_string()),
        };
        let lazy = AuthContext::deferred(recorded.clone(), || {
            panic!("recorded properties must not resolve the credential")
        });

        assert_eq!(lazy.handle(), Some("cached-user".into()));
        assert_eq!(lazy.cached_facts(), recorded);
    }

    #[test]
    fn handle_is_none_without_a_known_identity_and_does_not_load_credentials() {
        let deferred = AuthContext::deferred(logged_out(), || {
            panic!("reading an optional handle must not load the credential")
        });
        for context in [
            deferred,
            AuthContext::default(),
            AuthContext::new_from_token(Some("flox_pat_unknown-handle")),
            AuthContext::new_from_token(Some("flox_sat_unknown-handle")),
            AuthContext::from_bare_token(test_bare_token("unknown-handle")),
            AuthContext::new_kerberos(),
        ] {
            assert_eq!(context.handle(), None);
        }
    }

    /// Kerberos carries no bearer secret, so it records `logged_in: false` —
    /// and without `requires_login` gating the answer that would read as "not
    /// logged in" for a mode where logging in is not a thing.
    #[test]
    fn kerberos_is_not_unauthenticated_despite_having_no_secret() {
        let facts = AuthContext::from_kerberos(None).cached_facts();

        assert_eq!(facts, CachedFacts {
            logged_in: false,
            kind: CredentialKind::Kerberos,
            requires_login: false,
            expires_at: None,
            subject: None,
            handle: None,
            fingerprint: None,
        });
        assert!(!facts.is_unauthenticated());

        let lazy = AuthContext::deferred(facts, || {
            panic!("the recorded facts answer without acquiring a ticket")
        });
        assert!(!lazy.is_unauthenticated());
    }

    fn me_response(handle: &str) -> serde_json::Value {
        json!({"handle": handle, "user_id": "pat|test", "expires_at": null})
    }

    #[tokio::test]
    async fn identity_resolves_once_and_records_the_identity() {
        let server = MockServer::start();
        let mock = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me")
                .header("authorization", "bearer flox_pat_context-record");
            then.status(200).json_body(me_response("testuser"));
        });
        let client = FloxhubClient::new(client_config(&server.base_url())).unwrap();
        let records = Arc::new(Mutex::new(Vec::new()));
        let recorded = records.clone();
        let context = AuthContext::new_from_token(Some("flox_pat_context-record"))
            .with_identity_recorder(move |facts| recorded.lock().unwrap().push(facts.clone()));

        assert_eq!(context.handle(), None);
        let expected_identity = Some(UserIdentity {
            handle: "testuser".into(),
            sub: Some("pat|test".into()),
            expires_at: None,
        });
        assert_eq!(context.identity(&client).await.unwrap(), expected_identity);
        assert_eq!(context.handle(), Some("testuser".into()));
        assert_eq!(
            context.clone().identity(&client).await.unwrap(),
            expected_identity
        );
        assert_eq!(context.clone().handle(), Some("testuser".into()));
        mock.assert_calls(1);
        let expected = context.cached_facts();
        assert_eq!(*records.lock().unwrap(), vec![expected.clone(), expected]);
    }

    #[tokio::test]
    async fn refresh_replaces_a_cached_handle() {
        let server = MockServer::start();
        let mut stale = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(200).json_body(me_response("old-handle"));
        });
        let client = FloxhubClient::new(client_config(&server.base_url())).unwrap();
        let context = AuthContext::new_from_token(Some("flox_pat_context-refresh"));

        assert_eq!(context.handle(), None);
        context.identity(&client).await.unwrap().unwrap();
        assert_eq!(context.handle(), Some("old-handle".into()));
        stale.delete();
        let fresh = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(200).json_body(me_response("new-handle"));
        });
        assert_eq!(context.handle(), Some("old-handle".into()));
        fresh.assert_calls(0);
        assert_eq!(
            context.refresh_identity(&client).await.unwrap(),
            Some(UserIdentity {
                handle: "new-handle".into(),
                sub: Some("pat|test".into()),
                expires_at: None,
            })
        );
        assert_eq!(context.handle(), Some("new-handle".into()));
        fresh.assert_calls(1);
    }

    #[tokio::test]
    async fn failed_identity_lookups_are_retried_and_never_recorded() {
        let server = MockServer::start();
        let mut failure = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(500);
        });
        let client = FloxhubClient::new(client_config(&server.base_url())).unwrap();
        let context = AuthContext::new_from_token(Some("flox_pat_context-failure"))
            .with_identity_recorder(|_| panic!("failed lookups must not be recorded"));

        assert_eq!(context.identity(&client).await.unwrap(), None);
        assert_eq!(context.identity(&client).await.unwrap(), None);
        failure.assert_calls(2);
        failure.delete();
        server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me");
            then.status(401);
        });
        assert!(matches!(
            context.identity(&client).await,
            Err(AuthFailure::TokenExpired)
        ));
    }

    #[tokio::test]
    async fn identity_checks_the_record_against_the_loaded_credential() {
        let server = MockServer::start();
        let request = server.mock(|when, then| {
            when.method(httpmock::Method::GET)
                .path("/accounts/api/v1/accounts/me")
                .header("authorization", "bearer flox_pat_context-replaced");
            then.status(200).json_body(me_response("current-user"));
        });
        let client = FloxhubClient::new(client_config(&server.base_url())).unwrap();
        let stored = AuthContext::new_from_token(Some("flox_pat_context-matched"));
        let facts = CachedFacts {
            handle: Some("recorded-user".into()),
            subject: Some("pat|recorded".into()),
            ..stored.cached_facts()
        };
        let matched = AuthContext::deferred(facts.clone(), move || stored.clone());
        matched.identity(&client).await.unwrap().unwrap();
        assert_eq!(matched.handle(), Some("recorded-user".into()));
        request.assert_calls(0);

        let replaced = AuthContext::deferred(facts, || {
            AuthContext::new_from_token(Some("flox_pat_context-replaced"))
        });
        assert_eq!(replaced.handle(), Some("recorded-user".into()));
        replaced.token_secret();
        assert_eq!(replaced.handle(), None);
        replaced.identity(&client).await.unwrap().unwrap();
        assert_eq!(replaced.handle(), Some("current-user".into()));
        request.assert_calls(1);
    }
}
