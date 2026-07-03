use std::sync::LazyLock;

use itertools::Itertools;
use thiserror::Error;
use tracing::debug;
use url::Url;

/// Compiled-in default FloxHub base, used when no `floxhub_url` is configured.
///
/// Defaults to the hosted base `https://hub.flox.dev`. An on-premise build may
/// recompile this to its own base; [`Floxhub`] derives the git and API URLs
/// from whichever base is in effect, so changing the default does not affect
/// hosted-realm detection, which keys on host shape.
pub static DEFAULT_FLOXHUB_URL: LazyLock<Url> =
    LazyLock::new(|| Url::parse("https://hub.flox.dev").unwrap());

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
    /// The FloxHub API endpoint — the base the catalog client is configured
    /// with, onto which it joins `/api/v1/catalog/...`. It is served from the
    /// service root, so hosted resolves to `api.<...>.flox.dev/` and enterprise
    /// / on-premise to the base root `<base>/` (no extra path segment: a `/api`
    /// segment here would double with the client's path to `<base>/api/api/...`).
    const API: Self = Transform {
        saas_prefix: "api",
        saas_path: "",
        path: "",
    };
    /// The FloxHub git endpoint. Hosted resolves to `api.<...>.flox.dev/git`,
    /// enterprise / on-premise to `<base>/git`.
    const GIT: Self = Transform {
        saas_prefix: "api",
        saas_path: "git",
        path: "git",
    };
}

#[derive(Debug, Clone)]
pub struct Floxhub {
    base_url: Url,
    api_url: Url,
    git_url: Url,
    git_url_overridden: bool,
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

    /// The API base as a string with any trailing slash removed, for a client
    /// that joins paths by concatenation
    /// (`format!("{base}/api/v1/catalog/...")`). A bare-host [`Url`] always
    /// serializes with a trailing `/`, which would otherwise yield
    /// `<base>//api/v1/catalog`; trim it here so the join is clean.
    pub fn api_url_str(&self) -> String {
        self.api_url.as_str().trim_end_matches('/').to_string()
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
    /// An empty `route` appends nothing and just drops a trailing empty segment,
    /// so the base itself is the target (used by the API endpoint, which lives
    /// at the service root).
    ///
    /// Errors only for a cannot-be-a-base URL (e.g. `mailto:`); valid http(s)
    /// bases always succeed.
    pub(crate) fn route_url(base_url: &Url, route: &str) -> Result<Url, FloxhubError> {
        let mut url = base_url.clone();
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|()| FloxhubError::CannotBeABase(base_url.to_string()))?;
            segments.pop_if_empty();
            if !route.is_empty() {
                segments.push(route);
            }
        }
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

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

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

    // The API endpoint is the catalog client's base; the client joins
    // `/api/v1/catalog/...` onto it. It therefore lives at the service root, so
    // enterprise must be `<base>` (NOT `<base>/api`, which would double to
    // `<base>/api/api/v1/catalog`) and the trailing slash is trimmed by
    // `api_url_str` before the join.

    #[test]
    fn resolve_effective_url_api_hosted_base_is_api_subdomain_root() {
        let base = Url::from_str("https://hub.flox.dev").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::API, None)
                .unwrap()
                .as_str(),
            "https://api.flox.dev/",
        );
    }

    #[test]
    fn resolve_effective_url_api_staging_base_preserves_intermediate_labels() {
        let base = Url::from_str("https://hub.staging.flox.dev").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::API, None)
                .unwrap()
                .as_str(),
            "https://api.staging.flox.dev/",
        );
    }

    #[test]
    fn resolve_effective_url_api_enterprise_is_base_root_not_slash_api() {
        let base = Url::from_str("https://onprem.example.internal").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::API, None)
                .unwrap()
                .as_str(),
            "https://onprem.example.internal/",
        );
    }

    #[test]
    fn resolve_effective_url_api_enterprise_preserves_path_prefix() {
        let base = Url::from_str("https://host.internal/floxhub/").unwrap();
        assert_eq!(
            Floxhub::resolve_effective_url(&base, Transform::API, None)
                .unwrap()
                .as_str(),
            "https://host.internal/floxhub",
        );
    }

    #[test]
    fn api_url_str_trims_trailing_slash_for_clean_catalog_join() {
        // The catalog endpoint is `api_url_str() + "/api/v1/catalog/..."`.
        // Hosted and enterprise must both join without a doubled `/` or `/api`.
        let hosted = Floxhub::new(Url::from_str("https://hub.flox.dev").unwrap(), None, None)
            .unwrap()
            .api_url_str();
        assert_eq!(hosted, "https://api.flox.dev");
        assert_eq!(
            format!("{hosted}/api/v1/catalog/build-inputs/lookup"),
            "https://api.flox.dev/api/v1/catalog/build-inputs/lookup",
        );

        let enterprise = Floxhub::new(
            Url::from_str("https://onprem.example.internal/").unwrap(),
            None,
            None,
        )
        .unwrap()
        .api_url_str();
        assert_eq!(enterprise, "https://onprem.example.internal");
        assert_eq!(
            format!("{enterprise}/api/v1/catalog/build-inputs/lookup"),
            "https://onprem.example.internal/api/v1/catalog/build-inputs/lookup",
        );
    }
}
