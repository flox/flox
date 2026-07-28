//! GitHub release/commit resolver and clone delegate.
//!
//! `GitHubSource` is a concrete struct (not a trait) — see Design
//! Constraint #3 and CLAUDE.md provider-trait guidance. P04 may extract
//! a trait if a second source materializes.
//!
//! **Test-only override:** `FLOX_EXTENSIONS_GITHUB_BASE_URL` overrides the
//! default `https://api.github.com` for bats fixtures and integration
//! tests. Not exposed as a CLI flag or config key.

use std::io::Write;
use std::path::Path;
use std::time::Duration;

use flox_rust_sdk::providers::git::{GitCommandProvider, GitRemoteCommandError};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::info;

use super::manifest::AuthorManifest;

const DEFAULT_BASE_URL: &str = "https://api.github.com";
const BASE_URL_ENV_VAR: &str = "FLOX_EXTENSIONS_GITHUB_BASE_URL";

/// Progress template for `download_asset` when Content-Length is known.
/// `{msg}` is set by the caller to the asset filename so that concurrent
/// or sequential downloads (e.g. `upgrade --all`) are distinguishable on
/// the user's terminal.
pub(super) const PROGRESS_TEMPLATE: &str = "{spinner} {msg} {bytes}/{total_bytes} [{bar:30}] {eta}";

/// Progress template for `download_asset` when Content-Length is absent
/// (spinner fallback). Omits `{total_bytes}`, `{bar}`, and `{eta}` — none
/// of which render meaningfully without a known total.
pub(super) const SPINNER_TEMPLATE: &str = "{spinner} {msg} {bytes} ({bytes_per_sec})";

/// Resolved GitHub reference: a full commit SHA plus the human-readable
/// tag/branch coordinates that produced it (used to pick the clone ref
/// and to populate `state.toml`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedRef {
    pub commit: String,
    pub tag: Option<String>,
    pub branch: Option<String>,
}

/// Sort order for `search_repos`. The GitHub Search API accepts `stars`
/// or `updated`; both are requested in descending order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSort {
    Stars,
    Updated,
}

impl SearchSort {
    fn as_str(self) -> &'static str {
        match self {
            SearchSort::Stars => "stars",
            SearchSort::Updated => "updated",
        }
    }
}

/// Parameters for [`GitHubSource::search_repos`]. The topic filter
/// `topic:flox-extension archived:false` is implicit; user-supplied
/// `query` and `owner` are appended as additional qualifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchQuery {
    pub query: Option<String>,
    pub owner: Option<String>,
    pub limit: u8,
    pub sort: SearchSort,
}

impl SearchQuery {
    /// Clamp `limit` into `1..=100` (the GitHub Search API maximum
    /// `per_page`).
    pub fn new(query: Option<String>, owner: Option<String>, limit: u8, sort: SearchSort) -> Self {
        let limit = limit.clamp(1, 100);
        Self {
            query,
            owner,
            limit,
            sort,
        }
    }
}

#[derive(Debug, Error)]
pub enum GitHubError {
    #[error("HTTP {status} for {url}")]
    HttpStatus { status: u16, url: String },
    #[error("HTTP {status} downloading asset {url}")]
    AssetHttpStatus { status: u16, url: String },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("github resource not found: {0}")]
    NotFound(String),
    #[error("invalid git ref '{0}': contains characters not allowed in a ref")]
    InvalidRef(String),
    #[error("malformed github response from {url}: {detail}")]
    Malformed { url: String, detail: String },
    #[error("failed to write asset to {path}: {source}")]
    AssetIo {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("github rate limited ({status}); set GH_TOKEN or GITHUB_TOKEN to raise the limit")]
    RateLimited { status: u16 },
    #[error("github auth failed ({status}); check GH_TOKEN / GITHUB_TOKEN")]
    AuthFailed { status: u16 },
    #[error("refused to download asset from untrusted host '{host}' ({url})")]
    UnsafeAssetHost { host: String, url: String },
    #[error("malformed asset download url '{url}'")]
    UnparseableAssetUrl { url: String },
}

/// Owner (user or organization) validation failure for
/// [`validate_owner`]. Kept separate from [`GitHubError`] because it
/// describes a client-side input problem, not a transport/API failure.
#[derive(Debug, Error, PartialEq, Eq)]
#[error(
    "invalid owner '{0}': must be 1-39 characters of letters, digits, or hyphens, \
     with no leading/trailing or consecutive hyphens"
)]
pub struct InvalidOwner(pub String);

/// Enforce GitHub login rules on a `--owner` argument so it cannot smuggle
/// additional search qualifiers (e.g. `x user:attacker`) into the `q=`
/// parameter. GitHub logins are 1–39 chars of `[A-Za-z0-9-]` with no
/// leading/trailing or consecutive hyphens.
pub fn validate_owner(owner: &str) -> Result<(), InvalidOwner> {
    let len = owner.len();
    if !(1..=39).contains(&len) {
        return Err(InvalidOwner(owner.to_string()));
    }
    let bytes = owner.as_bytes();
    if bytes[0] == b'-' || bytes[len - 1] == b'-' {
        return Err(InvalidOwner(owner.to_string()));
    }
    let mut prev_hyphen = false;
    for &b in bytes {
        let is_hyphen = b == b'-';
        let valid = b.is_ascii_alphanumeric() || is_hyphen;
        if !valid || (is_hyphen && prev_hyphen) {
            return Err(InvalidOwner(owner.to_string()));
        }
        prev_hyphen = is_hyphen;
    }
    Ok(())
}

/// Allow-list check used by [`GitHubSource::check_asset_host`]. Matches
/// `host` exactly against `github.com` / `githubusercontent.com`, against
/// any subdomain of either, and against the host component of
/// `base_url` (to cover the `FLOX_EXTENSIONS_GITHUB_BASE_URL` test
/// override). Case-insensitive.
fn host_allowed(host: &str, base_url: &str) -> bool {
    const APEX: &[&str] = &["github.com", "githubusercontent.com"];
    let host_lc = host.to_ascii_lowercase();
    for apex in APEX {
        if host_lc == *apex || host_lc.ends_with(&format!(".{apex}")) {
            return true;
        }
    }
    if let Ok(base) = url::Url::parse(base_url)
        && let Some(base_host) = base.host_str()
        && host_lc == base_host.to_ascii_lowercase()
    {
        return true;
    }
    false
}

/// A single release asset (binary attached to a release).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub content_type: String,
}

#[derive(Debug, Clone)]
pub struct GitHubSource {
    client: reqwest::Client,
    base_url: String,
    auth_token: Option<String>,
}

impl GitHubSource {
    /// Constructor for tests that supply a mock HTTP client and base URL.
    /// Production code uses [`Self::from_env`].
    #[cfg(test)]
    pub(crate) fn new(client: reqwest::Client, base_url: String) -> Self {
        Self {
            client,
            base_url,
            auth_token: None,
        }
    }

    /// Build a `GitHubSource` with the default reqwest client and the
    /// `FLOX_EXTENSIONS_GITHUB_BASE_URL` env override (test-only) honored.
    /// Also reads `GH_TOKEN` / `GITHUB_TOKEN` from the environment so the
    /// Search API moves from 10 req/min to 30 req/min when a token is set.
    pub fn from_env() -> Self {
        let base_url =
            std::env::var(BASE_URL_ENV_VAR).unwrap_or_else(|_| DEFAULT_BASE_URL.to_string());
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .user_agent("flox-extensions")
            .build()
            .expect("failed to build reqwest client for GitHubSource");
        Self {
            client,
            base_url,
            auth_token: auth_token_from_env(),
        }
    }

    /// Inject an auth token post-construction (tests bypass env). Empty
    /// strings are coerced to `None` so the caller can't accidentally
    /// send `Authorization: Bearer ` and trigger a confusing 401.
    #[cfg(test)]
    pub(crate) fn with_auth_token(mut self, token: Option<String>) -> Self {
        self.auth_token = token.filter(|t| !t.is_empty());
        self
    }

    /// A GET builder for a GitHub **API** URL, carrying
    /// `Authorization: Bearer <token>` when one is configured. This lifts
    /// the anonymous rate limit and allows private repos.
    ///
    /// It is deliberately NOT used for asset downloads: the token must not
    /// ride a redirect to an off-`api.github.com` CDN host.
    fn api_get(&self, url: &str) -> reqwest::RequestBuilder {
        let req = self.client.get(url);
        match self.auth_token.as_deref() {
            Some(token) => req.bearer_auth(token),
            None => req,
        }
    }

    /// Clone `https://github.com/<owner>/<repo>.git` into `dest` at
    /// `branch_or_ref` (single-branch, no tags). The clone URL ignores
    /// `base_url` because that override only applies to the API; for
    /// integration tests, the `git config url.<base>.insteadOf` mechanism
    /// (set up by the bats fixture) redirects the clone.
    pub fn clone_repo(
        &self,
        owner: &str,
        repo: &str,
        branch_or_ref: &str,
        dest: &Path,
    ) -> Result<(), GitRemoteCommandError> {
        let url = format!("https://github.com/{owner}/{repo}.git");
        GitCommandProvider::clone_branch(&url, dest, branch_or_ref, false)?;
        Ok(())
    }

    /// Map an unsuccessful, non-404 HTTP response into the most specific
    /// `GitHubError` variant the caller should surface. 401 becomes
    /// `AuthFailed` (user hasn't set a valid `GH_TOKEN`/`GITHUB_TOKEN`);
    /// 429 and 403-with-`x-ratelimit-remaining: 0` become `RateLimited`;
    /// any other non-success code falls through to a generic `HttpStatus`.
    ///
    /// 404 stays per-site because each call carries its own "not found"
    /// phrasing (owner/repo vs. tag vs. ref). Callers must handle it
    /// before invoking this helper.
    fn classify_http_error(
        status: reqwest::StatusCode,
        headers: &reqwest::header::HeaderMap,
        url: String,
    ) -> GitHubError {
        let code = status.as_u16();
        let rate_limit_exhausted = headers
            .get("x-ratelimit-remaining")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim() == "0")
            .unwrap_or(false);
        match code {
            401 => GitHubError::AuthFailed { status: code },
            403 if rate_limit_exhausted => GitHubError::RateLimited { status: code },
            429 => GitHubError::RateLimited { status: code },
            _ => GitHubError::HttpStatus { status: code, url },
        }
    }

    /// Latest release (preferred) or default-branch HEAD (fallback).
    pub async fn resolve_latest(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<ResolvedRef, GitHubError> {
        let url = format!("{}/repos/{owner}/{repo}/releases/latest", self.base_url);
        let resp = self.api_get(&url).send().await?;
        let status = resp.status();
        if status.is_success() {
            let body: ReleaseBody = resp.json().await.map_err(|e| GitHubError::Malformed {
                url: url.clone(),
                detail: e.to_string(),
            })?;
            // tag_name is the human ref; target_commitish is a branch or commit.
            // Resolve the tag to a commit SHA via /commits/<tag_name>.
            let commit = self.resolve_commit(owner, repo, &body.tag_name).await?;
            return Ok(ResolvedRef {
                commit,
                tag: Some(body.tag_name),
                branch: None,
            });
        }
        if status.as_u16() != 404 {
            return Err(Self::classify_http_error(status, resp.headers(), url));
        }

        // Fallback: default branch HEAD.
        let repo_url = format!("{}/repos/{owner}/{repo}", self.base_url);
        let resp = self.api_get(&repo_url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            if status.as_u16() == 404 {
                return Err(GitHubError::NotFound(format!("{owner}/{repo}")));
            }
            return Err(Self::classify_http_error(status, resp.headers(), repo_url));
        }
        let body: RepoBody = resp.json().await.map_err(|e| GitHubError::Malformed {
            url: repo_url.clone(),
            detail: e.to_string(),
        })?;
        let commit = self
            .resolve_commit(owner, repo, &body.default_branch)
            .await?;
        Ok(ResolvedRef {
            commit,
            tag: None,
            branch: Some(body.default_branch),
        })
    }

    /// Resolve a user-supplied pin: tag (`v*` / semver-ish) or commit-SHA prefix.
    pub async fn resolve_pin(
        &self,
        owner: &str,
        repo: &str,
        pin: &str,
    ) -> Result<ResolvedRef, GitHubError> {
        if pin.is_empty() {
            return Err(GitHubError::NotFound(pin.to_string()));
        }
        // The pin is interpolated into API URL paths/queries; reject any ref
        // with characters that could alter the URL structure (spaces, `#`,
        // `?`, `&`, `%`, control chars) rather than encode them. Real tags,
        // branches, and SHAs only use `[A-Za-z0-9._/-]`.
        if !is_url_safe_ref(pin) {
            return Err(GitHubError::InvalidRef(pin.to_string()));
        }
        if looks_like_tag(pin) {
            let url = format!("{}/repos/{owner}/{repo}/releases/tags/{pin}", self.base_url);
            let resp = self.api_get(&url).send().await?;
            let status = resp.status();
            if status.as_u16() == 404 {
                return Err(GitHubError::NotFound(format!(
                    "tag '{pin}' on {owner}/{repo}"
                )));
            }
            if !status.is_success() {
                return Err(Self::classify_http_error(status, resp.headers(), url));
            }
            let body: ReleaseBody = resp.json().await.map_err(|e| GitHubError::Malformed {
                url: url.clone(),
                detail: e.to_string(),
            })?;
            let commit = self.resolve_commit(owner, repo, &body.tag_name).await?;
            return Ok(ResolvedRef {
                commit,
                tag: Some(body.tag_name),
                branch: None,
            });
        }
        if is_hex_prefix(pin) {
            // Resolve the commit AND the default branch in parallel: the
            // commit is what we pin to, but we need a branch name to drive
            // the `git clone --branch <ref>` step (git won't clone by raw
            // SHA). The caller resets to `commit` after the clone lands.
            let commit = self.resolve_commit(owner, repo, pin).await?;
            let branch = self.fetch_default_branch(owner, repo).await?;
            return Ok(ResolvedRef {
                commit,
                tag: None,
                branch: Some(branch),
            });
        }
        // Fall back to treating `pin` as a branch (or other ref) name: the
        // commits endpoint accepts any ref that resolves on the remote.
        match self.resolve_commit(owner, repo, pin).await {
            Ok(commit) => Ok(ResolvedRef {
                commit,
                tag: None,
                branch: Some(pin.to_string()),
            }),
            Err(GitHubError::NotFound(_)) => Err(GitHubError::NotFound(format!(
                "pin '{pin}' on {owner}/{repo}; expected a tag (e.g. 'v1.2.3'), a commit SHA prefix, or a branch name"
            ))),
            Err(err) => Err(err),
        }
    }

    /// `GET /repos/:owner/:repo` and extract `default_branch`.
    async fn fetch_default_branch(&self, owner: &str, repo: &str) -> Result<String, GitHubError> {
        let url = format!("{}/repos/{owner}/{repo}", self.base_url);
        let resp = self.api_get(&url).send().await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(GitHubError::NotFound(format!("{owner}/{repo}")));
        }
        if !status.is_success() {
            return Err(Self::classify_http_error(status, resp.headers(), url));
        }
        let body: RepoBody = resp.json().await.map_err(|e| GitHubError::Malformed {
            url: url.clone(),
            detail: e.to_string(),
        })?;
        Ok(body.default_branch)
    }

    /// `GET /repos/:owner/:repo/releases/tags/:tag` and return the `assets[]`
    /// array. Returns an empty vec if the release exists but has no assets;
    /// returns `NotFound` if the tag is not a published release.
    pub async fn list_release_assets(
        &self,
        owner: &str,
        repo: &str,
        tag: &str,
    ) -> Result<Vec<ReleaseAsset>, GitHubError> {
        let url = format!("{}/repos/{owner}/{repo}/releases/tags/{tag}", self.base_url);
        let resp = self.api_get(&url).send().await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(GitHubError::NotFound(format!(
                "tag '{tag}' on {owner}/{repo}"
            )));
        }
        if !status.is_success() {
            return Err(Self::classify_http_error(status, resp.headers(), url));
        }
        let body: ReleaseAssetsBody = resp.json().await.map_err(|e| GitHubError::Malformed {
            url: url.clone(),
            detail: e.to_string(),
        })?;
        Ok(body.assets)
    }

    /// Reject asset download URLs whose host is not on the allowlist.
    ///
    /// `browser_download_url` is server-supplied (via the GitHub releases
    /// API JSON) and could in principle point at any host. We restrict to:
    ///
    /// * `github.com` and subdomains of `github.com` — release-asset
    ///   redirect origin.
    /// * `githubusercontent.com` and subdomains (e.g.
    ///   `objects.githubusercontent.com`) — signed-URL CDN GitHub redirects
    ///   real downloads through.
    /// * The host of `self.base_url` — covers the test override where
    ///   `FLOX_EXTENSIONS_GITHUB_BASE_URL` points at a local mock server.
    fn check_asset_host(&self, asset_url: &str) -> Result<(), GitHubError> {
        let parsed = url::Url::parse(asset_url).map_err(|_| GitHubError::UnparseableAssetUrl {
            url: asset_url.to_string(),
        })?;
        let host = parsed
            .host_str()
            .ok_or_else(|| GitHubError::UnparseableAssetUrl {
                url: asset_url.to_string(),
            })?;
        if host_allowed(host, &self.base_url) {
            return Ok(());
        }
        Err(GitHubError::UnsafeAssetHost {
            host: host.to_string(),
            url: asset_url.to_string(),
        })
    }

    /// Stream `asset.browser_download_url` into `dest`, computing a SHA-256
    /// digest along the way. Returns the hex-encoded digest.
    ///
    /// Uses `reqwest::Response::bytes_stream()` so the full body is never
    /// held in memory and the checksum is computed in the same pass that
    /// writes to disk.
    pub async fn download_asset(
        &self,
        asset: &ReleaseAsset,
        dest: &Path,
    ) -> Result<String, GitHubError> {
        let url = &asset.browser_download_url;
        self.check_asset_host(url)?;
        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(GitHubError::AssetHttpStatus {
                status: status.as_u16(),
                url: url.clone(),
            });
        }
        let total = resp.content_length();
        let bar = match total {
            Some(n) if n > 0 => {
                let bar = ProgressBar::new(n);
                bar.set_style(
                    ProgressStyle::with_template(PROGRESS_TEMPLATE)
                        .unwrap_or_else(|_| ProgressStyle::default_bar()),
                );
                bar
            },
            _ => {
                let bar = ProgressBar::new_spinner();
                bar.set_style(
                    ProgressStyle::with_template(SPINNER_TEMPLATE)
                        .unwrap_or_else(|_| ProgressStyle::default_spinner()),
                );
                bar
            },
        };
        bar.set_message(asset.name.clone());
        let mut file = std::fs::File::create(dest).map_err(|source| GitHubError::AssetIo {
            path: dest.display().to_string(),
            source,
        })?;
        let mut hasher = Sha256::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let bytes = chunk?;
            hasher.update(&bytes);
            file.write_all(&bytes)
                .map_err(|source| GitHubError::AssetIo {
                    path: dest.display().to_string(),
                    source,
                })?;
            bar.inc(bytes.len() as u64);
        }
        bar.finish_and_clear();
        file.flush().map_err(|source| GitHubError::AssetIo {
            path: dest.display().to_string(),
            source,
        })?;
        let digest = hasher.finalize();
        Ok(hex_encode(&digest))
    }

    /// `GET /search/repositories?q=topic:flox-extension ...` — used by
    /// `flox extension search`. The topic filter and `archived:false`
    /// qualifier are always included; user-supplied `query` and `owner`
    /// are appended. When [`Self::auth_token`] is set the request carries
    /// `Authorization: Bearer <token>`, lifting the anonymous Search API
    /// quota from 10 req/min to 30 req/min.
    pub async fn search_repos(&self, q: &SearchQuery) -> Result<SearchResponse, GitHubError> {
        let url = format!("{}/search/repositories", self.base_url);
        let q_param = build_search_query(q);
        let mut req = self
            .client
            .get(&url)
            .query(&[
                ("q", q_param.as_str()),
                ("sort", q.sort.as_str()),
                ("order", "desc"),
            ])
            .query(&[("per_page", q.limit)]);
        if let Some(token) = self.auth_token.as_deref() {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await?;
        let status = resp.status();
        let code = status.as_u16();
        if status.is_success() {
            let body: SearchResponse = resp.json().await.map_err(|e| GitHubError::Malformed {
                url: url.clone(),
                detail: e.to_string(),
            })?;
            return Ok(body);
        }
        if code == 404 {
            return Err(GitHubError::NotFound(url));
        }
        Err(Self::classify_http_error(status, resp.headers(), url))
    }

    /// `GET /repos/:owner/:repo/commits/:ref` and extract the full SHA.
    async fn resolve_commit(
        &self,
        owner: &str,
        repo: &str,
        r#ref: &str,
    ) -> Result<String, GitHubError> {
        let url = format!("{}/repos/{owner}/{repo}/commits/{}", self.base_url, r#ref);
        let resp = self.api_get(&url).send().await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(GitHubError::NotFound(format!(
                "ref '{}' on {owner}/{repo}",
                r#ref
            )));
        }
        if !status.is_success() {
            return Err(Self::classify_http_error(status, resp.headers(), url));
        }
        let body: CommitBody = resp.json().await.map_err(|e| GitHubError::Malformed {
            url: url.clone(),
            detail: e.to_string(),
        })?;
        if body.sha.is_empty() {
            return Err(GitHubError::Malformed {
                url,
                detail: "empty sha".to_string(),
            });
        }
        Ok(body.sha)
    }

    /// `GET /repos/:owner/:repo/contents/flox-extension.toml?ref=<ref>` with
    /// `Accept: application/vnd.github.raw` to fetch the author manifest at
    /// a specific commit/tag. Returns `Ok(None)` when the file doesn't
    /// exist (manifest is optional); parse failures bubble as
    /// `GitHubError::Malformed`.
    pub async fn fetch_author_manifest(
        &self,
        owner: &str,
        repo: &str,
        r#ref: &str,
    ) -> Result<Option<AuthorManifest>, GitHubError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/contents/flox-extension.toml?ref={}",
            self.base_url, r#ref
        );
        let resp = self
            .api_get(&url)
            .header("Accept", "application/vnd.github.raw")
            .send()
            .await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(GitHubError::HttpStatus {
                status: status.as_u16(),
                url,
            });
        }
        let body = resp.text().await.map_err(|e| GitHubError::Malformed {
            url: url.clone(),
            detail: e.to_string(),
        })?;
        let manifest =
            super::manifest::parse_author_manifest(&body).map_err(|e| GitHubError::Malformed {
                url,
                detail: e.to_string(),
            })?;
        Ok(Some(manifest))
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseBody {
    tag_name: String,
}

#[derive(Debug, Deserialize)]
struct RepoBody {
    default_branch: String,
}

#[derive(Debug, Deserialize)]
struct CommitBody {
    sha: String,
}

#[derive(Debug, Deserialize)]
struct ReleaseAssetsBody {
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

/// Raw GitHub Search API response. `incomplete_results` flips to `true`
/// when the server truncates the result set (usually due to rate-limit or
/// time-budget pressure); surfaced to the user as a stderr warning.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SearchResponse {
    #[serde(default)]
    pub total_count: u64,
    #[serde(default)]
    pub incomplete_results: bool,
    #[serde(default)]
    pub items: Vec<SearchItem>,
}

/// One repository in a search response.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SearchItem {
    pub full_name: String,
    pub owner: SearchOwner,
    pub name: String,
    #[serde(default)]
    pub stargazers_count: u64,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub html_url: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct SearchOwner {
    pub login: String,
}

/// Error returned by [`resolve_asset`] when no asset matches the host's
/// platform. The caller converts this into an `InstallError::NoMatchingAsset`
/// so the user-facing hint (mentioning the repo) lives with the other
/// install errors.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoMatchingAssetError {
    pub platform: String,
}

/// Return the list of lowercase substrings that indicate a release asset
/// matches the current host. `os` is one of `linux`/`darwin`/`windows`;
/// `arch` is one of `x86_64`/`aarch64` (other targets fall back to the
/// arch name as-is).
///
/// Authors ship assets under a variety of conventions (`x86_64` vs `amd64`,
/// `aarch64` vs `arm64`, `darwin` vs `macos`), so both aliases are emitted
/// and the matcher walks them in priority order.
pub(crate) fn platform_matchers_for(os: &str, arch: &str) -> Vec<String> {
    let os_aliases: &[&str] = match os {
        "macos" | "darwin" => &["darwin", "macos"],
        "linux" => &["linux"],
        "windows" => &["windows"],
        _ => return Vec::new(),
    };
    let arch_aliases: &[&str] = match arch {
        "x86_64" | "amd64" => &["x86_64", "amd64"],
        "aarch64" | "arm64" => &["aarch64", "arm64"],
        other => {
            // Best effort: emit just the single token so odd archs still match
            // `flox-<name>-<os>-<arch>.tar.gz` when authors follow the naming
            // convention directly.
            let mut out = Vec::with_capacity(os_aliases.len());
            for o in os_aliases {
                out.push(format!("{o}-{other}"));
            }
            return out;
        },
    };
    let mut out = Vec::with_capacity(os_aliases.len() * arch_aliases.len());
    for o in os_aliases {
        for a in arch_aliases {
            out.push(format!("{o}-{a}"));
        }
    }
    out
}

/// Resolve which release asset to download for the host running flox, given
/// a list of release assets, an optional author manifest (for template /
/// explicit mapping), and the extension `name`. Priority order:
///
/// 1. *(Reserved for future `BinaryMeta.platforms`)* — not yet implemented.
/// 2. `manifest.extension.binary.asset` rendered with `{name}`, `{os}`,
///    `{arch}` placeholders; exact asset-name match.
/// 3. Substring match against [`platform_matchers_for`] on the asset name.
/// 4. Rosetta fallback: on `darwin-aarch64`, retry as `darwin-x86_64`; if
///    matched, emit `tracing::info!` noting the fallback.
///
/// On exhaustion returns the host's matcher string so the caller can
/// produce a helpful `no release asset matches ...` error.
pub(crate) fn resolve_asset<'a>(
    assets: &'a [ReleaseAsset],
    manifest: Option<&AuthorManifest>,
    name: &str,
) -> Result<&'a ReleaseAsset, NoMatchingAssetError> {
    resolve_asset_for(
        assets,
        manifest,
        name,
        std::env::consts::OS,
        std::env::consts::ARCH,
    )
}

/// Host-aware variant of [`resolve_asset`] for unit tests.
pub(crate) fn resolve_asset_for<'a>(
    assets: &'a [ReleaseAsset],
    manifest: Option<&AuthorManifest>,
    name: &str,
    os: &str,
    arch: &str,
) -> Result<&'a ReleaseAsset, NoMatchingAssetError> {
    if let Some(binary) = manifest.and_then(|m| m.extension.binary.as_ref()) {
        let tmpl = binary.asset.trim();
        if !tmpl.is_empty() {
            let rendered = render_asset_template(tmpl, name, os, arch);
            if let Some(a) = assets.iter().find(|a| a.name == rendered) {
                return Ok(a);
            }
        }
    }

    let matchers = platform_matchers_for(os, arch);
    if let Some(a) = substring_match(assets, &matchers) {
        return Ok(a);
    }

    // Rosetta fallback: apple silicon can run x86_64 darwin binaries.
    if (os == "macos" || os == "darwin") && (arch == "aarch64" || arch == "arm64") {
        let fallback = platform_matchers_for(os, "x86_64");
        if let Some(a) = substring_match(assets, &fallback) {
            info!(
                asset = %a.name,
                "no arm64 release asset; falling back to x86_64 under Rosetta"
            );
            return Ok(a);
        }
    }

    Err(NoMatchingAssetError {
        platform: matchers
            .into_iter()
            .next()
            .unwrap_or_else(|| format!("{os}-{arch}")),
    })
}

/// Checksum / signature / provenance sidecars that sit next to a real
/// release binary and must never be selected as the executable — they
/// contain a platform substring too (e.g. `flox-x-linux-x86_64.tar.gz.sha256`).
fn is_sidecar_asset(name: &str) -> bool {
    const SIDECAR_SUFFIXES: &[&str] = &[
        ".sha256",
        ".sha512",
        ".sha1",
        ".md5",
        ".sig",
        ".asc",
        ".pem",
        ".sbom",
        ".intoto.jsonl",
    ];
    let lower = name.to_ascii_lowercase();
    SIDECAR_SUFFIXES.iter().any(|s| lower.ends_with(s))
}

fn substring_match<'a>(
    assets: &'a [ReleaseAsset],
    matchers: &[String],
) -> Option<&'a ReleaseAsset> {
    for needle in matchers {
        if let Some(a) = assets
            .iter()
            .find(|a| a.name.contains(needle) && !is_sidecar_asset(&a.name))
        {
            return Some(a);
        }
    }
    None
}

fn render_asset_template(tmpl: &str, name: &str, os: &str, arch: &str) -> String {
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    tmpl.replace("{name}", name)
        .replace("{os}", os)
        .replace("{arch}", arch)
        .replace("{ext}", ext)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Read `GH_TOKEN` then `GITHUB_TOKEN` from the environment, returning the
/// first non-empty value. Empty strings are treated as unset so users can
/// clear the token with `GH_TOKEN=`.
fn auth_token_from_env() -> Option<String> {
    for var in ["GH_TOKEN", "GITHUB_TOKEN"] {
        if let Ok(v) = std::env::var(var)
            && !v.is_empty()
        {
            return Some(v);
        }
    }
    None
}

/// Compose the `q=` parameter for `search_repos`. Always prefixes
/// `topic:flox-extension archived:false`; appends user-supplied query
/// text (free-form) and a `user:<owner>` qualifier when present. Empty
/// tokens are dropped so trailing whitespace never leaks into the URL.
fn build_search_query(q: &SearchQuery) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(4);
    parts.push("topic:flox-extension".to_string());
    parts.push("archived:false".to_string());
    if let Some(text) = q.query.as_deref() {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            parts.push(trimmed.to_string());
        }
    }
    if let Some(owner) = q.owner.as_deref() {
        let trimmed = owner.trim();
        if !trimmed.is_empty() {
            parts.push(format!("user:{trimmed}"));
        }
    }
    parts.join(" ")
}

fn is_hex_prefix(s: &str) -> bool {
    !s.is_empty() && s.len() <= 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Heuristic: tag-like if it starts with `v` followed by a digit, or if
/// the first char is a digit (e.g. `1.2.3`) AND it contains a `.`.
/// Otherwise treat as a commit prefix.
fn looks_like_tag(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if first == 'v' {
        return chars.next().is_some_and(|c| c.is_ascii_digit());
    }
    first.is_ascii_digit() && s.contains('.')
}

/// True if `r` contains only characters safe to interpolate into a URL
/// path or query without encoding: `[A-Za-z0-9._/-]`. Real tags, branches,
/// and SHAs stay within this set; anything else (spaces, `#`, `?`, `&`,
/// `%`, control characters) is rejected rather than encoded.
fn is_url_safe_ref(r: &str) -> bool {
    !r.is_empty()
        && r.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '-'))
}

#[cfg(test)]
mod tests {
    use httpmock::Method::GET;
    use httpmock::MockServer;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    use super::*;

    fn fixture_source(server: &MockServer) -> GitHubSource {
        GitHubSource::new(reqwest::Client::new(), server.base_url())
    }

    #[tokio::test]
    async fn resolve_latest_returns_release_tag_when_release_exists() {
        let server = MockServer::start_async().await;
        let release_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/latest");
            then.status(200).json_body(json!({
                "tag_name": "v1.2.3",
                "target_commitish": "main"
            }));
        });
        let commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/v1.2.3");
            then.status(200).json_body(json!({
                "sha": "abc1234567890abcdef1234567890abcdef123456"
            }));
        });

        let source = fixture_source(&server);
        let resolved = source.resolve_latest("owner", "flox-hello").await.unwrap();

        assert_eq!(resolved, ResolvedRef {
            commit: "abc1234567890abcdef1234567890abcdef123456".to_string(),
            tag: Some("v1.2.3".to_string()),
            branch: None,
        });
        release_mock.assert_async().await;
        commit_mock.assert_async().await;
    }

    #[tokio::test]
    async fn resolve_latest_falls_back_to_default_branch_head_when_no_release() {
        let server = MockServer::start_async().await;
        let _release_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/latest");
            then.status(404);
        });
        let _repo_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/owner/flox-hello");
            then.status(200).json_body(json!({
                "default_branch": "trunk"
            }));
        });
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/trunk");
            then.status(200).json_body(json!({
                "sha": "deadbeefcafef00d0000000000000000feedface"
            }));
        });

        let source = fixture_source(&server);
        let resolved = source.resolve_latest("owner", "flox-hello").await.unwrap();

        assert_eq!(resolved, ResolvedRef {
            commit: "deadbeefcafef00d0000000000000000feedface".to_string(),
            tag: None,
            branch: Some("trunk".to_string()),
        });
    }

    #[tokio::test]
    async fn resolve_latest_returns_not_found_when_repo_missing() {
        let server = MockServer::start_async().await;
        let _release_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/latest");
            then.status(404);
        });
        let _repo_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/owner/flox-hello");
            then.status(404);
        });

        let source = fixture_source(&server);
        let err = source
            .resolve_latest("owner", "flox-hello")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    #[tokio::test]
    async fn resolve_pin_treats_v_prefix_as_tag() {
        let server = MockServer::start_async().await;
        let _tag_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/tags/v1.0.0");
            then.status(200).json_body(json!({ "tag_name": "v1.0.0" }));
        });
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/v1.0.0");
            then.status(200).json_body(json!({
                "sha": "1111111111111111111111111111111111111111"
            }));
        });

        let source = fixture_source(&server);
        let resolved = source
            .resolve_pin("owner", "flox-hello", "v1.0.0")
            .await
            .unwrap();
        assert_eq!(resolved, ResolvedRef {
            commit: "1111111111111111111111111111111111111111".to_string(),
            tag: Some("v1.0.0".to_string()),
            branch: None,
        });
    }

    #[tokio::test]
    async fn resolve_pin_treats_hex_as_commit_prefix() {
        let server = MockServer::start_async().await;
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/abc1234");
            then.status(200).json_body(json!({
                "sha": "abc1234567890abcdef1234567890abcdef123456"
            }));
        });
        // Hex pins also fetch the default branch so install_github has a
        // clonable ref name.
        let _repo_mock = server.mock(|when, then| {
            when.method(GET).path("/repos/owner/flox-hello");
            then.status(200)
                .json_body(json!({ "default_branch": "main" }));
        });

        let source = fixture_source(&server);
        let resolved = source
            .resolve_pin("owner", "flox-hello", "abc1234")
            .await
            .unwrap();
        assert_eq!(resolved, ResolvedRef {
            commit: "abc1234567890abcdef1234567890abcdef123456".to_string(),
            tag: None,
            branch: Some("main".to_string()),
        });
    }

    #[tokio::test]
    async fn resolve_pin_returns_err_when_tag_not_found() {
        let server = MockServer::start_async().await;
        let _tag_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/tags/v9.9.9");
            then.status(404);
        });

        let source = fixture_source(&server);
        let err = source
            .resolve_pin("owner", "flox-hello", "v9.9.9")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    /// BUG-14 regression: a pin that is neither tag-like nor a hex SHA
    /// should fall back to resolving as a branch (or other ref) name via
    /// the commits endpoint.
    #[tokio::test]
    async fn resolve_pin_resolves_branch_name_via_commits() {
        let server = MockServer::start_async().await;
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/main");
            then.status(200).json_body(json!({
                "sha": "2222222222222222222222222222222222222222"
            }));
        });

        let source = fixture_source(&server);
        let resolved = source
            .resolve_pin("owner", "flox-hello", "main")
            .await
            .unwrap();
        assert_eq!(resolved, ResolvedRef {
            commit: "2222222222222222222222222222222222222222".to_string(),
            tag: None,
            branch: Some("main".to_string()),
        });
    }

    /// BUG-14 regression: an unresolvable pin should surface the error
    /// with a hint listing the three supported pin shapes.
    #[tokio::test]
    async fn resolve_pin_unknown_ref_returns_hint_with_supported_shapes() {
        let server = MockServer::start_async().await;
        let _commit_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/commits/does-not-exist");
            then.status(404);
        });

        let source = fixture_source(&server);
        let err = source
            .resolve_pin("owner", "flox-hello", "does-not-exist")
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            matches!(err, GitHubError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
        assert!(
            msg.contains("tag") && msg.contains("commit") && msg.contains("branch"),
            "error message should mention all three pin shapes: {msg}"
        );
    }

    #[test]
    fn looks_like_tag_classifications() {
        assert!(looks_like_tag("v1.0.0"));
        assert!(looks_like_tag("v2"));
        assert!(looks_like_tag("1.2.3"));
        assert!(!looks_like_tag("abc1234"));
        assert!(!looks_like_tag(""));
        assert!(!looks_like_tag("vfoo"));
        assert!(!looks_like_tag("123abc")); // hex-like, not a tag
    }

    #[test]
    fn is_hex_prefix_classifications() {
        assert!(is_hex_prefix("abc"));
        assert!(is_hex_prefix("0123456789abcdef"));
        assert!(!is_hex_prefix(""));
        assert!(!is_hex_prefix("xyz"));
        assert!(!is_hex_prefix("abc-def"));
        // Exactly 40 chars: ok; 41: rejected.
        assert!(is_hex_prefix(&"a".repeat(40)));
        assert!(!is_hex_prefix(&"a".repeat(41)));
    }

    fn mk_asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.to_string(),
            browser_download_url: format!("https://example.invalid/{name}"),
            size: 0,
            content_type: "application/octet-stream".to_string(),
        }
    }

    #[tokio::test]
    async fn list_release_assets_returns_assets_for_tag() {
        let server = MockServer::start_async().await;
        let _tag_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/tags/v1.0.0");
            then.status(200).json_body(json!({
                "tag_name": "v1.0.0",
                "assets": [
                    {
                        "name": "flox-hello-linux-x86_64.tar.gz",
                        "browser_download_url": "https://example.invalid/1.tar.gz",
                        "size": 100,
                        "content_type": "application/gzip"
                    },
                    {
                        "name": "flox-hello-darwin-aarch64.tar.gz",
                        "browser_download_url": "https://example.invalid/2.tar.gz",
                        "size": 200,
                        "content_type": "application/gzip"
                    }
                ]
            }));
        });
        let source = fixture_source(&server);
        let assets = source
            .list_release_assets("owner", "flox-hello", "v1.0.0")
            .await
            .unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(assets[0].name, "flox-hello-linux-x86_64.tar.gz");
        assert_eq!(assets[1].name, "flox-hello-darwin-aarch64.tar.gz");
    }

    #[tokio::test]
    async fn list_release_assets_empty_when_release_has_no_assets() {
        let server = MockServer::start_async().await;
        let _tag_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/tags/v1.0.0");
            then.status(200).json_body(json!({
                "tag_name": "v1.0.0"
            }));
        });
        let source = fixture_source(&server);
        let assets = source
            .list_release_assets("owner", "flox-hello", "v1.0.0")
            .await
            .unwrap();
        assert!(assets.is_empty());
    }

    #[tokio::test]
    async fn list_release_assets_not_found_when_tag_missing() {
        let server = MockServer::start_async().await;
        let _tag_mock = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/owner/flox-hello/releases/tags/v9.9.9");
            then.status(404);
        });
        let source = fixture_source(&server);
        let err = source
            .list_release_assets("owner", "flox-hello", "v9.9.9")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubError::NotFound(_)),
            "expected NotFound, got {err:?}"
        );
    }

    // TS01 — resolve_asset priority table: manifest template beats substring,
    // substring beats Rosetta, Rosetta fallback on darwin-aarch64.
    #[test]
    fn resolve_asset_template_beats_substring() {
        let assets = vec![
            mk_asset("flox-hello-linux-x86_64.tar.gz"),
            mk_asset("custom-linux-x86_64.tar.gz"),
        ];
        let manifest = AuthorManifest {
            schema: "1".to_string(),
            extension: super::super::manifest::ExtensionMeta {
                name: "hello".to_string(),
                description: None,
                binary: Some(super::super::manifest::BinaryMeta {
                    source: "github-release".to_string(),
                    asset: "custom-{os}-{arch}.tar.gz".to_string(),
                    sha256: None,
                }),
            },
            environment: None,
            on_active: None,
        };
        let chosen =
            resolve_asset_for(&assets, Some(&manifest), "hello", "linux", "x86_64").unwrap();
        assert_eq!(chosen.name, "custom-linux-x86_64.tar.gz");
    }

    #[test]
    fn resolve_asset_substring_match_when_no_template() {
        let assets = vec![
            mk_asset("flox-hello-darwin-x86_64.tar.gz"),
            mk_asset("flox-hello-linux-x86_64.tar.gz"),
        ];
        let chosen = resolve_asset_for(&assets, None, "hello", "linux", "x86_64").unwrap();
        assert_eq!(chosen.name, "flox-hello-linux-x86_64.tar.gz");
    }

    #[test]
    fn resolve_asset_substring_accepts_amd64_alias() {
        let assets = vec![mk_asset("flox-hello-linux-amd64.tar.gz")];
        let chosen = resolve_asset_for(&assets, None, "hello", "linux", "x86_64").unwrap();
        assert_eq!(chosen.name, "flox-hello-linux-amd64.tar.gz");
    }

    #[test]
    fn resolve_asset_substring_accepts_arm64_alias() {
        let assets = vec![mk_asset("flox-hello-darwin-arm64.tar.gz")];
        let chosen = resolve_asset_for(&assets, None, "hello", "darwin", "aarch64").unwrap();
        assert_eq!(chosen.name, "flox-hello-darwin-arm64.tar.gz");
    }

    #[test]
    fn resolve_asset_rosetta_fallback_on_darwin_arm64() {
        let assets = vec![mk_asset("flox-hello-darwin-x86_64.tar.gz")];
        let chosen = resolve_asset_for(&assets, None, "hello", "darwin", "aarch64").unwrap();
        assert_eq!(chosen.name, "flox-hello-darwin-x86_64.tar.gz");
    }

    #[test]
    fn resolve_asset_no_match_on_unavailable_platform() {
        let assets = vec![mk_asset("flox-hello-windows-x86_64.zip")];
        let err = resolve_asset_for(&assets, None, "hello", "linux", "x86_64").unwrap_err();
        assert_eq!(err, NoMatchingAssetError {
            platform: "linux-x86_64".to_string(),
        });
    }

    // TS02 — platform_matchers_for returns the expected strings.
    #[test]
    fn platform_matchers_for_linux_x86_64() {
        assert_eq!(platform_matchers_for("linux", "x86_64"), vec![
            "linux-x86_64".to_string(),
            "linux-amd64".to_string()
        ],);
    }

    #[test]
    fn resolve_asset_skips_checksum_sidecar() {
        // The sidecar sorts first and also contains the platform substring;
        // selection must skip it and choose the real archive.
        let assets = vec![
            mk_asset("flox-hi-linux-x86_64.tar.gz.sha256"),
            mk_asset("flox-hi-linux-x86_64.tar.gz"),
        ];
        let chosen = resolve_asset_for(&assets, None, "hi", "linux", "x86_64").unwrap();
        assert_eq!(chosen.name, "flox-hi-linux-x86_64.tar.gz");
    }

    #[test]
    fn is_sidecar_asset_matches_common_suffixes() {
        assert!(is_sidecar_asset("x-linux-x86_64.tar.gz.sha256"));
        assert!(is_sidecar_asset("x.sig"));
        assert!(is_sidecar_asset("X.ASC"));
        assert!(!is_sidecar_asset("flox-hi-linux-x86_64.tar.gz"));
        assert!(!is_sidecar_asset("flox-hi-linux-x86_64"));
    }

    #[test]
    fn is_url_safe_ref_classifications() {
        for ok in ["v1.2.3", "main", "feature/x", "abc1234", "v2-rc.1", "1.0.0"] {
            assert!(is_url_safe_ref(ok), "expected safe: {ok:?}");
        }
        for bad in ["", "v1#rc1", "a?b", "a&b", "a b", "a%20b", "a\tb", "a\nb"] {
            assert!(!is_url_safe_ref(bad), "expected unsafe: {bad:?}");
        }
    }

    #[tokio::test]
    async fn resolve_pin_rejects_url_unsafe_ref() {
        // No server needed: the ref is rejected before any HTTP call.
        let source = GitHubSource::new(reqwest::Client::new(), "http://127.0.0.1:1/".to_string());
        let err = source
            .resolve_pin("owner", "flox-hi", "v1#rc1")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubError::InvalidRef(_)),
            "expected InvalidRef, got {err:?}"
        );
    }

    #[tokio::test]
    async fn resolve_latest_sends_authorization_header_when_token_set() {
        // Proves the token is attached to non-search API calls, not just
        // search. Both requests (releases/latest and commits/<tag>) must
        // carry the header, so the mocks require it.
        let server = MockServer::start_async().await;
        let _release = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/o/r/releases/latest")
                .header("authorization", "Bearer tok-123");
            then.status(200)
                .json_body(json!({ "tag_name": "v1.0.0", "target_commitish": "main" }));
        });
        let _commit = server.mock(|when, then| {
            when.method(GET)
                .path("/repos/o/r/commits/v1.0.0")
                .header("authorization", "Bearer tok-123");
            then.status(200).json_body(json!({
                "sha": "abc1234567890abcdef1234567890abcdef123456"
            }));
        });
        let source = fixture_source(&server).with_auth_token(Some("tok-123".to_string()));
        let resolved = source.resolve_latest("o", "r").await.unwrap();
        assert_eq!(resolved.tag.as_deref(), Some("v1.0.0"));
    }

    #[test]
    fn platform_matchers_for_darwin_aarch64() {
        assert_eq!(platform_matchers_for("darwin", "aarch64"), vec![
            "darwin-aarch64".to_string(),
            "darwin-arm64".to_string(),
            "macos-aarch64".to_string(),
            "macos-arm64".to_string(),
        ],);
    }

    #[test]
    fn platform_matchers_for_unknown_os_is_empty() {
        assert_eq!(
            platform_matchers_for("plan9", "x86_64"),
            Vec::<String>::new()
        );
    }

    // TS04 — download_asset streams bytes and computes a matching SHA-256.
    #[tokio::test]
    async fn download_asset_streams_and_computes_sha256() {
        let payload: Vec<u8> = (0u8..=63u8).collect();
        let expected_sha = {
            let mut h = Sha256::new();
            h.update(&payload);
            hex_encode(&h.finalize())
        };

        let server = MockServer::start_async().await;
        let _asset_mock = server.mock(|when, then| {
            when.method(GET).path("/asset/file.bin");
            then.status(200)
                .header("Content-Type", "application/octet-stream")
                .body(&payload);
        });

        let asset = ReleaseAsset {
            name: "file.bin".to_string(),
            browser_download_url: format!("{}/asset/file.bin", server.base_url()),
            size: payload.len() as u64,
            content_type: "application/octet-stream".to_string(),
        };

        let source = fixture_source(&server);
        let temp = tempfile::TempDir::new().unwrap();
        let dest = temp.path().join("file.bin");
        let sha = source.download_asset(&asset, &dest).await.unwrap();
        assert_eq!(sha, expected_sha);
        assert_eq!(std::fs::read(&dest).unwrap(), payload);
    }

    #[tokio::test]
    async fn download_asset_refuses_off_allowlist_host() {
        let server = MockServer::start_async().await;
        let source = fixture_source(&server);
        let asset = ReleaseAsset {
            name: "evil.bin".to_string(),
            browser_download_url: "http://169.254.169.254/evil.bin".to_string(),
            size: 0,
            content_type: "application/octet-stream".to_string(),
        };
        let temp = tempfile::TempDir::new().unwrap();
        let dest = temp.path().join("evil.bin");
        let err = source.download_asset(&asset, &dest).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::UnsafeAssetHost { .. }),
            "expected UnsafeAssetHost, got {err:?}"
        );
        assert!(!dest.exists());
    }

    #[test]
    fn host_allowed_matches_apex_subdomains_and_base_url() {
        assert!(host_allowed("github.com", "https://example.test/"));
        assert!(host_allowed("objects.githubusercontent.com", "https://x/"));
        assert!(host_allowed("raw.githubusercontent.com", "https://x/"));
        // Case-insensitive.
        assert!(host_allowed("GitHub.COM", "https://x/"));
        // Subdomain confusion must not slip through.
        assert!(!host_allowed("github.com.evil.corp", "https://x/"));
        assert!(!host_allowed("notgithub.com", "https://x/"));
        // Matches base_url host for test overrides (e.g. mock servers).
        assert!(host_allowed("127.0.0.1", "http://127.0.0.1:12345/"));
    }

    #[test]
    fn progress_bar_template_includes_message_placeholder() {
        // Both templates must carry a `{msg}` placeholder so callers can
        // inject the asset name. Without this, `upgrade --all` prints
        // indistinguishable progress lines for each binary.
        for tpl in [PROGRESS_TEMPLATE, SPINNER_TEMPLATE] {
            assert!(tpl.contains("{msg}"), "template missing {{msg}}: {tpl}");
        }
    }

    // TS01 — query-string composition with user-supplied query + owner.
    #[tokio::test]
    async fn search_repos_composes_topic_query_and_owner_filter() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET)
                .path("/search/repositories")
                .query_param("q", "topic:flox-extension archived:false hello user:acme")
                .query_param("sort", "stars")
                .query_param("order", "desc")
                .query_param("per_page", "25");
            then.status(200).json_body(json!({
                "total_count": 0,
                "incomplete_results": false,
                "items": []
            }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(
            Some("hello".to_string()),
            Some("acme".to_string()),
            25,
            SearchSort::Stars,
        );
        let _resp = source.search_repos(&q).await.unwrap();
    }

    // TS02 — empty query and owner produce only the topic + archived
    // qualifiers (no trailing tokens).
    #[tokio::test]
    async fn search_repos_omits_empty_tokens() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET)
                .path("/search/repositories")
                .query_param("q", "topic:flox-extension archived:false")
                .query_param("sort", "updated")
                .query_param("per_page", "50");
            then.status(200).json_body(json!({
                "total_count": 0,
                "incomplete_results": false,
                "items": []
            }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 50, SearchSort::Updated);
        let _resp = source.search_repos(&q).await.unwrap();
    }

    // TS03 — canned response with incomplete_results and a null description
    // deserializes into typed fields.
    #[tokio::test]
    async fn search_repos_parses_incomplete_results_and_null_description() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(200).json_body(json!({
                "total_count": 2,
                "incomplete_results": true,
                "items": [
                    {
                        "full_name": "alpha/flox-one",
                        "owner": { "login": "alpha" },
                        "name": "flox-one",
                        "stargazers_count": 42,
                        "description": "canonical reference",
                        "archived": false,
                        "html_url": "https://github.com/alpha/flox-one"
                    },
                    {
                        "full_name": "beta/flox-two",
                        "owner": { "login": "beta" },
                        "name": "flox-two",
                        "stargazers_count": 7,
                        "description": null,
                        "archived": false,
                        "html_url": "https://github.com/beta/flox-two"
                    }
                ]
            }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let resp = source.search_repos(&q).await.unwrap();
        assert_eq!(resp, SearchResponse {
            total_count: 2,
            incomplete_results: true,
            items: vec![
                SearchItem {
                    full_name: "alpha/flox-one".to_string(),
                    owner: SearchOwner {
                        login: "alpha".to_string(),
                    },
                    name: "flox-one".to_string(),
                    stargazers_count: 42,
                    description: Some("canonical reference".to_string()),
                    archived: false,
                    html_url: "https://github.com/alpha/flox-one".to_string(),
                },
                SearchItem {
                    full_name: "beta/flox-two".to_string(),
                    owner: SearchOwner {
                        login: "beta".to_string(),
                    },
                    name: "flox-two".to_string(),
                    stargazers_count: 7,
                    description: None,
                    archived: false,
                    html_url: "https://github.com/beta/flox-two".to_string(),
                },
            ],
        });
    }

    // TS04 — HTTP status mapping: 401→AuthFailed, 403/429→RateLimited,
    // 500→HttpStatus.
    #[tokio::test]
    async fn search_repos_maps_401_to_auth_failed() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(401)
                .json_body(json!({ "message": "Bad credentials" }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let err = source.search_repos(&q).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::AuthFailed { status: 401 }),
            "expected AuthFailed(401), got {err:?}",
        );
    }

    #[tokio::test]
    async fn search_repos_maps_403_with_exhausted_quota_to_rate_limited() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(403)
                .header("x-ratelimit-remaining", "0")
                .json_body(json!({ "message": "API rate limit exceeded" }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let err = source.search_repos(&q).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::RateLimited { status: 403 }),
            "expected RateLimited(403), got {err:?}",
        );
    }

    #[tokio::test]
    async fn search_repos_maps_403_without_quota_header_to_http_status() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(403)
                .json_body(json!({ "message": "Resource protected by SSO" }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let err = source.search_repos(&q).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::HttpStatus { status: 403, .. }),
            "expected HttpStatus(403) (non-rate-limit 403), got {err:?}",
        );
    }

    #[tokio::test]
    async fn search_repos_maps_429_to_rate_limited() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(429);
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let err = source.search_repos(&q).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::RateLimited { status: 429 }),
            "expected RateLimited(429), got {err:?}",
        );
    }

    #[tokio::test]
    async fn search_repos_maps_500_to_http_status() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(500);
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let err = source.search_repos(&q).await.unwrap_err();
        assert!(
            matches!(err, GitHubError::HttpStatus { status: 500, .. }),
            "expected HttpStatus(500), got {err:?}",
        );
    }

    // BUG-10 regression: every GitHub API call (not just search_repos)
    // must map 401 → AuthFailed, 403+x-ratelimit-remaining:0 → RateLimited,
    // 429 → RateLimited. Exercise this through `resolve_latest` and
    // `list_release_assets` so the unified classifier is demonstrably
    // wired into all sites.
    #[tokio::test]
    async fn resolve_latest_maps_401_to_auth_failed() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/repos/o/r/releases/latest");
            then.status(401);
        });
        let source = fixture_source(&server);
        let err = source.resolve_latest("o", "r").await.unwrap_err();
        assert!(
            matches!(err, GitHubError::AuthFailed { status: 401 }),
            "expected AuthFailed(401), got {err:?}",
        );
    }

    #[tokio::test]
    async fn resolve_latest_maps_429_to_rate_limited() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/repos/o/r/releases/latest");
            then.status(429);
        });
        let source = fixture_source(&server);
        let err = source.resolve_latest("o", "r").await.unwrap_err();
        assert!(
            matches!(err, GitHubError::RateLimited { status: 429 }),
            "expected RateLimited(429), got {err:?}",
        );
    }

    #[tokio::test]
    async fn list_release_assets_maps_403_with_exhausted_quota_to_rate_limited() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET).path("/repos/o/r/releases/tags/v1");
            then.status(403)
                .header("x-ratelimit-remaining", "0")
                .json_body(json!({ "message": "API rate limit exceeded" }));
        });
        let source = fixture_source(&server);
        let err = source
            .list_release_assets("o", "r", "v1")
            .await
            .unwrap_err();
        assert!(
            matches!(err, GitHubError::RateLimited { status: 403 }),
            "expected RateLimited(403), got {err:?}",
        );
    }

    // TS05 — when an auth token is set via `with_auth_token`, the request
    // carries `Authorization: Bearer <token>`. Uses the builder rather than
    // env mutation to avoid Rust 2024 `unsafe { set_var }` and inter-test
    // races.
    #[tokio::test]
    async fn search_repos_sends_authorization_header_when_token_set() {
        let server = MockServer::start_async().await;
        let _mock = server.mock(|when, then| {
            when.method(GET)
                .path("/search/repositories")
                .header("authorization", "Bearer test-token-xyz");
            then.status(200).json_body(json!({
                "total_count": 0,
                "incomplete_results": false,
                "items": []
            }));
        });
        let source = fixture_source(&server).with_auth_token(Some("test-token-xyz".to_string()));
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let _resp = source.search_repos(&q).await.unwrap();
    }

    #[tokio::test]
    async fn search_repos_omits_authorization_header_when_unset() {
        let server = MockServer::start_async().await;
        // Two mocks: a strict "with Authorization header" that must NOT fire,
        // and a default "no such header required" that must fire.
        let strict = server.mock(|when, then| {
            when.method(GET)
                .path("/search/repositories")
                .header_exists("authorization");
            then.status(500);
        });
        let fallback = server.mock(|when, then| {
            when.method(GET).path("/search/repositories");
            then.status(200).json_body(json!({
                "total_count": 0,
                "incomplete_results": false,
                "items": []
            }));
        });
        let source = fixture_source(&server);
        let q = SearchQuery::new(None, None, 30, SearchSort::Stars);
        let _resp = source.search_repos(&q).await.unwrap();
        strict.assert_calls_async(0).await;
        fallback.assert_async().await;
    }

    #[test]
    fn build_search_query_joins_tokens_with_spaces() {
        let q = SearchQuery::new(
            Some("  hello  ".to_string()),
            Some(" acme ".to_string()),
            10,
            SearchSort::Stars,
        );
        assert_eq!(
            build_search_query(&q),
            "topic:flox-extension archived:false hello user:acme"
        );
    }

    #[test]
    fn build_search_query_drops_empty_or_whitespace_tokens() {
        let q = SearchQuery::new(
            Some("   ".to_string()),
            Some(String::new()),
            10,
            SearchSort::Stars,
        );
        assert_eq!(
            build_search_query(&q),
            "topic:flox-extension archived:false"
        );
    }

    #[test]
    fn search_query_clamps_limit_to_max_100() {
        let q = SearchQuery::new(None, None, 250, SearchSort::Stars);
        assert_eq!(q.limit, 100);
        let q = SearchQuery::new(None, None, 0, SearchSort::Stars);
        assert_eq!(q.limit, 1);
    }

    #[test]
    fn validate_owner_accepts_normal_logins() {
        for ok in ["a", "acme", "flox-examples", "Acme-Org-99", &"a".repeat(39)] {
            validate_owner(ok).unwrap_or_else(|e| panic!("expected Ok for {ok:?}: {e}"));
        }
    }

    #[test]
    fn validate_owner_rejects_injection_and_edge_cases() {
        let bad = [
            "",                // empty
            "-lead",           // leading hyphen
            "trail-",          // trailing hyphen
            "double--hyphen",  // consecutive hyphens
            "x user:attacker", // injection attempt (space)
            "a/b",             // path component
            "acme\ttab",       // tab
            &"a".repeat(40),   // too long
            "dot.separated",   // dot not allowed
        ];
        for s in bad {
            assert_eq!(validate_owner(s), Err(InvalidOwner(s.to_string())));
        }
    }

    #[test]
    fn with_auth_token_filters_empty_string() {
        let src = GitHubSource::new(reqwest::Client::new(), "http://x".to_string())
            .with_auth_token(Some(String::new()));
        assert!(src.auth_token.is_none(), "empty token must be dropped");
        let src = GitHubSource::new(reqwest::Client::new(), "http://x".to_string())
            .with_auth_token(Some("abc".to_string()));
        assert_eq!(src.auth_token.as_deref(), Some("abc"));
    }

    #[test]
    fn spinner_template_omits_unknown_total_placeholders() {
        // Without a known Content-Length the bar/eta/total_bytes fields
        // would render as empty or zero, so the spinner template must not
        // reference them.
        let tpl = SPINNER_TEMPLATE;
        assert!(
            !tpl.contains("{total_bytes}"),
            "spinner template should not use total_bytes: {tpl}"
        );
        assert!(
            !tpl.contains("{bar"),
            "spinner template should not use a bar: {tpl}"
        );
        assert!(
            !tpl.contains("{eta"),
            "spinner template should not use eta: {tpl}"
        );
    }
}
