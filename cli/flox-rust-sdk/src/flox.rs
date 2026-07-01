use std::path::PathBuf;
use std::str::FromStr;
use std::sync::LazyLock;

use flox_core::vars::FLOX_VERSION_STRING;
pub use floxhub_client::{AuthContext, AuthnMode, FloxhubToken, FloxhubTokenError};
use floxhub_client::{FloxhubClient, FloxhubClientError};
use itertools::Itertools;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;
use url::Url;
use uuid::Uuid;

use crate::data::FloxVersion;
use crate::providers::flake_installable_locker;

pub static FLOX_VERSION: LazyLock<FloxVersion> = LazyLock::new(|| {
    let Ok(version) = (*FLOX_VERSION_STRING).parse() else {
        // Production builds won't panic since we run `flox --version` in pkgs/flox/default.nix.
        panic!(
            "Version '{version}' cannot be parsed",
            version = *FLOX_VERSION_STRING
        )
    };
    version
});

/// The main API struct for our flox implementation
///
/// A [Flox] instance serves as the context for nix invocations
/// and possibly other tools such as git.
/// As a CLI application one invocation of `flox` would run on the same instance
/// but may call different methods.
///
/// [Flox] will provide a preconfigured instance of the Nix API.
/// By default this nix API uses the nix CLI.
/// Preconfiguration includes environment variables and flox specific arguments.
#[derive(Debug)]
pub struct Flox {
    /// The directory pointing to the users flox configuration
    ///
    /// TODO: set a default in the lib or CLI?
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,
    pub state_dir: PathBuf,
    pub temp_dir: PathBuf,
    pub runtime_dir: PathBuf,

    pub system: String,
    pub system_user_name: String,
    pub system_hostname: String,
    // The command used to run flox
    pub argv: Vec<String>,

    pub floxhub: Floxhub,

    /// The current authentication credential.
    pub auth_context: AuthContext,

    /// Shared HTTP client for both the catalog and factory API surfaces.
    pub floxhub_client: FloxhubClient,

    pub installable_locker: flake_installable_locker::InstallableLockerImpl,

    /// Feature flags
    pub features: Features,

    pub verbosity: i32,

    /// Device UUID for telemetry correlation.
    /// None when metrics are disabled.
    pub metrics_device_uuid: Option<Uuid>,
}

impl Flox {
    /// Set a new token and rebuild the credential to reflect it.
    ///
    /// Note: when using Kerberos authentication, the token is stored but has
    /// no effect on the credential — Kerberos does not use FloxHub tokens.
    pub fn set_auth_context(
        &mut self,
        auth_context: AuthContext,
    ) -> Result<(), FloxhubClientError> {
        self.auth_context = auth_context.clone();
        self.floxhub_client.update_config(|config| {
            config.auth_context = auth_context;
        })?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Default)]
pub struct Features {
    #[serde(default)]
    pub qa: bool,
    #[serde(default)]
    pub beta: bool,
    #[serde(default)]
    pub auto_activate: bool,
}

/// Compiled-in default FloxHub base, used when no `floxhub_url` is configured.
///
/// Defaults to the hosted base `https://hub.flox.dev`. An on-premise build may
/// recompile this to its own base; [`Floxhub`] derives the git and API URLs
/// from whichever base is in effect, so changing the default does not affect
/// hosted-realm detection, which keys on host shape.
pub static DEFAULT_FLOXHUB_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://hub.flox.dev").unwrap());

#[derive(Debug, Clone)]
pub struct Floxhub {
    base_url: Url,
    api_url: Url,
    git_url: Url,
    git_url_overridden: bool,
}

/// How to derive one FloxHub component URL (git, API, ...) from the base URL.
///
/// FloxHub runs in two topologies, and every component URL is derived from the
/// single configured base URL according to which one applies:
///
/// - Hosted (SaaS): components live on sibling subdomains of the same
///   `*.flox.dev` zone. The web app is `hub.flox.dev`; git and the API are on
///   `api.flox.dev`. Staging and preview deployments keep the shape with an
///   extra label (`hub.preview.flox.dev` pairs with `api.preview.flox.dev`).
///   A component is reached by replacing the `hub` label with `saas_prefix`
///   and setting the path to `saas_path`.
/// - Enterprise / on-premise: one host serves everything, so a component is
///   reached by appending `path` to the base URL.
///
/// One `Transform` describes these knobs for a single component; see
/// [`Transform::GIT`]. `Floxhub::resolve_effective_url` selects the topology
/// from the base URL's host shape and applies the matching rule.
struct Transform<'a> {
    /// Subdomain label substituted for `hub` on the hosted service.
    saas_prefix: &'a str,
    /// Path set on the hosted service URL.
    saas_path: &'a str,
    /// Path segment appended to the base URL for enterprise / on-premise.
    path: &'a str,
}

impl Transform<'static> {
    /// The FloxHub api endpoint. Hosted resolves to `api.<...>.flox.dev/`,
    /// enterprise / on-premise to `<base>/git`.
    const API: Self = Transform {
        saas_prefix: "api",
        saas_path: "",
        path: "api",
    };
    /// The FloxHub git endpoint. Hosted resolves to `api.<...>.flox.dev/git`,
    /// enterprise / on-premise to `<base>/git`.
    const GIT: Self = Transform {
        saas_prefix: "api",
        saas_path: "git",
        path: "git",
    };
}

impl Floxhub {
    pub fn new(
        base_url: Url,
        api_url_override: Option<Url>,
        git_url_override: Option<Url>,
    ) -> Result<Self, FloxhubError> {
        let git_url_overridden = git_url_override.is_some();
        let git_url = Self::resolve_effective_url(&base_url, Transform::GIT, git_url_override)?;
        let api_url = Self::resolve_effective_url(&base_url, Transform::API, api_url_override)?;

        let hub = Floxhub {
            base_url,
            api_url,
            git_url,
            git_url_overridden,
        };

        debug!(?hub, "Determined FloxHub urls");

        Ok(hub)
    }

    /// Derive a component URL from `base_url`, applying `transform` for the
    /// topology `base_url` belongs to.
    ///
    /// Precedence:
    /// 1. An explicit `url_override` (e.g. `_FLOX_FLOXHUB_GIT_URL`) — used verbatim.
    /// 2. A hosted base, recognised by the host shape `hub.<...>.flox.dev`: the
    ///    `hub` label is replaced with `transform.saas_prefix` and any
    ///    intermediate labels are preserved (so staging and preview bases
    ///    resolve to their paired host), then the path is set to
    ///    `transform.saas_path`. This also covers existing managed-environment
    ///    pointers, which persist the hosted base.
    /// 3. Any other base: `transform.path` appended to the base on the same
    ///    host (enterprise / on-premise).
    ///
    /// Detection keys on the host shape, not an exact URL. A recompiled
    /// [`DEFAULT_FLOXHUB_URL`] outside `flox.dev` therefore routes off its own
    /// base instead of being rewritten to the hosted host, and hosted staging
    /// and preview subdomains resolve correctly. See the Enterprise On-Premise
    /// SL-003 design.
    fn resolve_effective_url(
        base_url: &Url,
        transform: Transform,
        url_override: Option<Url>,
    ) -> Result<Url, FloxhubError> {
        if let Some(url_override) = url_override {
            return Ok(url_override);
        }

        let host_components = base_url
            .host_str()
            .ok_or(FloxhubError::CannotBeABase(base_url.to_string()))?
            .split(".")
            .collect::<Vec<_>>();
        match host_components.as_slice() {
            ["hub", intermediate @ .., "flox", "dev"] => {
                let host: String = [&[transform.saas_prefix], intermediate, &["flox", "dev"]]
                    .into_iter()
                    .flatten()
                    .join(".");

                let mut url = base_url.clone();
                url.set_host(Some(&host)).unwrap();
                url.set_path(transform.saas_path);

                debug!(%base_url, transformed=%url, "Transformed Flox SaaS url");
                Ok(url)
            },
            _ => {
                let url = Self::route_url(base_url, transform.path)?;
                debug!(%base_url, transformed=%url, "Transformed Flox Enterprise url");
                Ok(url)
            },
        }
    }

    /// Return the base url of the FloxHub instance
    /// might change to a more specific url in the future
    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Return the url of the FloxHub api endpoint
    ///
    /// If the environment variable `FLOX_CATALOG_URL` is set,
    /// it will be used instead of the derived FloxHub host.
    /// This is useful for testing FloxHub locally.
    pub fn api_url(&self) -> &Url {
        &self.api_url
    }

    pub fn git_url_override(&self) -> Option<&Url> {
        self.git_url_overridden.then_some(&self.git_url)
    }

    /// Return the url of the FloxHub git interface
    ///
    /// If the environment variable `_FLOX_FLOXHUB_GIT_URL` is set,
    /// it will be used instead of the derived FloxHub host.
    /// This is useful for testing FloxHub locally.
    pub fn git_url(&self) -> &Url {
        &self.git_url
    }

    /// Append a path-segment `route` to `base_url`, preserving any existing
    /// path prefix and collapsing a trailing slash (e.g. `https://host/git`,
    /// or `https://host/floxhub/` + `git` -> `https://host/floxhub/git`).
    ///
    /// Errors only for a cannot-be-a-base URL (e.g. `mailto:`); valid http(s)
    /// bases always succeed.
    pub(crate) fn route_url(base_url: &Url, route: &str) -> Result<Url, FloxhubError> {
        let mut url = base_url.clone();
        url.path_segments_mut()
            .map_err(|()| FloxhubError::CannotBeABase(base_url.to_string()))?
            .pop_if_empty()
            .push(route);
        Ok(url)
    }
}

#[derive(Error, Debug)]
pub enum FloxhubError {
    #[error(
        "Invalid FloxHub URL: '{0}'. Expected a base URL with a host, e.g. 'https://floxhub.example'."
    )]
    CannotBeABase(String),
}

pub mod test_helpers {
    use std::fs;

    use flox_core::data::environment_ref::EnvironmentOwner;
    use tempfile::{TempDir, tempdir_in};

    use super::*;
    use crate::providers::catalog::test_helpers::UNIT_TEST_GENERATED;
    use crate::providers::flake_installable_locker::{
        InstallableLockerImpl,
        InstallableLockerMock,
    };
    use crate::providers::git::{GitCommandProvider, GitProvider};

    pub fn create_test_token(handle: &str) -> FloxhubToken {
        let my_claims = serde_json::json!({
        "https://flox.dev/handle": handle,
        "exp": 9999999999_i64
        });

        // my_claims is a struct that implements Serialize
        // This will create a JWT using HS256 as algorithm
        let token = jsonwebtoken::encode(
            &jsonwebtoken::Header::default(),
            &my_claims,
            &jsonwebtoken::EncodingKey::from_secret("secret".as_ref()),
        )
        .unwrap();

        FloxhubToken::from_str(&token).unwrap()
    }

    /// Set a pre-existing token on a [Flox] instance and rebuild the auth
    /// strategy so that `auth_context.handle()` and friends see it immediately.
    pub fn set_test_token(flox: &mut Flox, token: FloxhubToken) {
        let auth_context = AuthContext::from_mode(&AuthnMode::Auth0, Some(token));

        let _ = flox.set_auth_context(auth_context);
    }

    /// Set up test authentication on a [Flox] instance.
    ///
    /// Creates a test token for the given handle, sets it on the instance,
    /// and rebuilds the auth strategy so that `auth_context.handle()` and friends
    /// see the token immediately.
    pub fn set_test_auth(flox: &mut Flox, handle: &str) {
        set_test_token(flox, create_test_token(handle));
    }

    /// Describes which test user to load:
    /// - One that has an existing personal catalog and access to other test
    ///   catalogs.
    /// - No access to org catalogs, and a personal catalog that doesn't exist
    ///   yet.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum PublishTestUser {
        WithCatalogs,
        NoCatalogs,
    }

    pub fn test_token_from_floxhub_test_users_file(user: PublishTestUser) -> FloxhubToken {
        let idx = match user {
            PublishTestUser::WithCatalogs => 0,
            PublishTestUser::NoCatalogs => 1,
        };
        let test_user_file_path = UNIT_TEST_GENERATED
            .parent()
            .unwrap()
            .join("floxhub_test_users.json");
        let contents =
            std::fs::read_to_string(test_user_file_path).expect("couldn't open test user file");
        let json: serde_json::Value =
            serde_json::from_str(&contents).expect("couldn't parse test user file");
        let token = json
            .get(idx)
            .and_then(|obj| obj.get("token"))
            .expect("couldn't extract token from test user file")
            .as_str()
            .unwrap()
            .to_string();
        // Parse the token to extract claims (including exp) from the JWT
        FloxhubToken::from_str(&token).expect("couldn't parse test user token")
    }

    pub fn flox_instance() -> (Flox, TempDir) {
        flox_instance_with_optional_floxhub(None)
    }

    /// If owner is None, no mock FloxHub is setup.
    /// If it is Some, a mock FloxHub with a repo for that owner will be setup,
    /// but no other owners will work.
    pub fn flox_instance_with_optional_floxhub(
        owner: Option<&EnvironmentOwner>,
    ) -> (Flox, TempDir) {
        // Use /tmp instead of std::env::temp_dir since we store sockets in runtime_dir,
        // and std::env::temp_dir may return a path that is too long.
        let tempdir_handle = tempfile::tempdir_in(PathBuf::from("/tmp")).unwrap();

        let cache_dir = tempdir_handle.path().join("caches");
        let data_dir = tempdir_handle.path().join(".local/share/flox");
        let state_dir = tempdir_handle.path().join(".local/state/flox");
        let temp_dir = tempdir_handle.path().join("temp");
        let config_dir = tempdir_handle.path().join("config");
        let runtime_dir = tempdir_handle.path().join("run");

        std::fs::create_dir_all(&cache_dir).unwrap();
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&state_dir).unwrap();
        std::fs::create_dir_all(&temp_dir).unwrap();
        std::fs::create_dir_all(&config_dir).unwrap();
        std::fs::create_dir_all(&runtime_dir).unwrap();

        let git_url_override = owner.map(|owner| {
            let mock_floxhub_git_dir = tempdir_in(&temp_dir).unwrap().keep();
            let floxmeta_dir = mock_floxhub_git_dir.join(owner.as_str()).join("floxmeta");
            fs::create_dir_all(&floxmeta_dir).unwrap();
            GitCommandProvider::init(floxmeta_dir, true).unwrap();
            Url::from_directory_path(mock_floxhub_git_dir).unwrap()
        });

        let auth_context = AuthContext::from_mode(&AuthnMode::default(), None);

        let flox = Flox {
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            system_user_name: "its-a-me-mario".to_string(),
            system_hostname: "mushroom-kingdom".to_string(),
            argv: vec![],
            cache_dir,
            data_dir,
            state_dir,
            temp_dir,
            config_dir,
            runtime_dir,
            floxhub: Floxhub::new(
                Url::from_str("https://hub.flox.dev").unwrap(),
                None,
                git_url_override,
            )
            .unwrap(),
            auth_context,
            floxhub_client: floxhub_client::client::test_helpers::new_noop(),
            installable_locker: InstallableLockerImpl::Mock(InstallableLockerMock::new()),
            features: Default::default(),
            verbosity: 0,
            metrics_device_uuid: None,
        };

        (flox, tempdir_handle)
    }
}

#[cfg(test)]
pub mod tests {
    use std::str::FromStr;

    use super::*;

    /// A fake FloxHub token
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 9999999999,                // 2286-11-20T17:46:39+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    const FAKE_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74";

    /// A fake floxhub token, that is expired
    ///
    /// {
    ///  "typ": "JWT",
    ///  "alg": "HS256"
    /// }
    /// .
    /// {
    ///   "https://flox.dev/handle": "test"
    ///   "exp": 1704063600,                // 2024-01-01T00:00:00+00:00
    /// }
    /// .
    /// AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
    const FAKE_EXPIRED_TOKEN: &str = "eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjE3MDQwNjM2MDB9.-5VCofPtmYQuvh21EV1nEJhTFV_URkRP0WFu4QDPFxY";

    #[tokio::test]
    async fn test_get_username() {
        let token = FloxhubToken::new(FAKE_TOKEN.to_string()).unwrap();
        assert_eq!(token.handle(), "test");
    }

    #[tokio::test]
    async fn test_detect_expired() {
        let token =
            FloxhubToken::new(FAKE_EXPIRED_TOKEN.to_string()).expect("Expired token should parse");
        assert!(token.is_expired(), "Token should be marked as expired");
        assert_eq!(token.handle(), "test", "Handle should still be accessible");
        assert!(
            !token.secret().is_empty(),
            "secret() should still return the token string for expired tokens"
        );
    }

    #[test]
    fn resolve_effective_url_uses_override_verbatim() {
        let base = Url::from_str("https://flox.example.internal").unwrap();
        let override_url = Url::from_str("https://git.is.cool.example.internal/git").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::GIT, Some(override_url.clone()))
                .unwrap(),
            override_url,
        );
    }

    #[test]
    fn resolve_effective_url_hosted_base_rewrites_hub_to_saas_host() {
        // The hosted realm splits its components across sibling subdomains, so
        // the `hub` label is rewritten to the transform's SaaS prefix and the
        // path is replaced.
        let base = Url::from_str("https://hub.flox.dev").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::GIT, None)
                .unwrap()
                .as_str(),
            "https://api.flox.dev/git",
        );
    }

    #[test]
    fn resolve_effective_url_hosted_base_preserves_intermediate_labels() {
        // A staging or preview base keeps its intermediate labels; only the
        // `hub` prefix is swapped for the SaaS prefix.
        let base = Url::from_str("https://hub.staging.flox.dev").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::GIT, None)
                .unwrap()
                .as_str(),
            "https://api.staging.flox.dev/git",
        );
    }

    #[test]
    fn resolve_effective_url_saas_pattern_is_anchored() {
        // SaaS detection is anchored on both ends: the host must start with
        // `hub` and end with `flox.dev`. A near-miss on either anchor is
        // treated as on-prem and routes off its own base.
        for host in ["nothub.flox.dev", "hub.flox.example.com"] {
            let base = Url::from_str(&format!("https://{host}")).unwrap();
            assert_eq!(
                Floxhub::resolve_effective_url(&base, Transform::GIT, None)
                    .unwrap()
                    .as_str(),
                format!("https://{host}/git"),
            );
        }
    }

    #[test]
    fn resolve_effective_url_other_base_routes_off_base() {
        assert_eq!(
            Floxhub::resolve_effective_url(
                &Url::from_str("https://flox.example.internal").unwrap(),
                Transform::GIT,
                None,
            )
            .unwrap(),
            Url::from_str("https://flox.example.internal/git").unwrap(),
        );
    }

    #[test]
    fn resolve_effective_url_routes_off_base_even_when_it_is_the_default() {
        // An on-premise build may recompile DEFAULT_FLOXHUB_URL to its own
        // base. SaaS detection matches the `hub.*.flox.dev` host shape, so a
        // base outside that shape routes off itself — it must NOT be rewritten
        // to the hosted SaaS host just because it equals the compiled default.
        let recompiled_default = Url::from_str("https://onprem.example.internal").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&recompiled_default, Transform::GIT, None)
                .unwrap()
                .as_str(),
            "https://onprem.example.internal/git",
        );
    }

    #[test]
    fn floxhub_new_routes_onprem_pointer_to_base_git() {
        // Mirrors floxmeta `open_at` reconstructing a Floxhub from a managed-
        // environment pointer whose base is an on-prem host (with the trailing
        // slash the pointer persists) and no git override.
        let base = Url::from_str("https://onprem.example.internal/").unwrap();
        let floxhub = Floxhub::new(base, None, None).unwrap();
        assert_eq!(
            floxhub.git_url().as_str(),
            "https://onprem.example.internal/git",
        );
    }

    #[test]
    fn route_url_preserves_path_prefix_and_trailing_slash() {
        assert_eq!(
            Floxhub::route_url(
                &Url::from_str("https://host.internal/floxhub/").unwrap(),
                "git"
            )
            .unwrap(),
            Url::from_str("https://host.internal/floxhub/git").unwrap(),
        );
    }
}
