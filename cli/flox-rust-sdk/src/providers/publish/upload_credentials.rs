//! Upload credentials that outlive FloxHub's short STS sessions.
//!
//! `nix copy` resolves S3 credentials through an AWS profile whose
//! `credential_process` is `flox publish-credentials --dir <dir>`. Nix 2.33+
//! runs that process for every S3 request (each multipart part included),
//! never caches the answer, and ignores `Expiration`. The helper therefore
//! owns caching and expiry: it serves the cached set from `<dir>` and asks
//! FloxHub for a fresh one only when fewer than [RENEWAL_MARGIN] remain.
//!
//! The helper never fails the process. When Nix's credential process fails,
//! Nix hides its message, falls through the AWS chain to EC2 instance
//! metadata, and on an EC2 runner could pick up the instance role. Instead
//! the helper records the reason in `<dir>/renewal-error` and hands out
//! placeholder credentials that S3 rejects, and `flox publish` reports the
//! recorded reason.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use serde::{Deserialize, Serialize};
use tempfile::{NamedTempFile, TempDir};
use tracing::debug;
use url::Url;

use super::PublishError;
use crate::providers::nix::nix_base_command;
use crate::providers::nix_auth::{CatalogAuth, catalog_auth_to_envs};

/// The AWS profile `nix copy` is pointed at.
pub const PROFILE: &str = "flox-publish";

/// Renew when fewer than this many minutes of a session remain. A retried
/// S3 request reuses the credentials of its first attempt, so the margin
/// must cover one part's upload plus Nix's retry backoff.
pub const RENEWAL_MARGIN: Duration = Duration::minutes(5);

/// The lifetime assumed for a session FloxHub hands out without an expiry:
/// STS's minimum, which is also FloxHub's default for ingress sessions.
const ASSUMED_SESSION_LIFETIME: Duration = Duration::seconds(900);

/// The expiry, if FloxHub ever includes one in the session's envs.
const EXPIRATION_ENV: &str = "AWS_CREDENTIAL_EXPIRATION";

/// 64 MiB parts: a 20 GB output is ~300 parts rather than ~4,000.
const MULTIPART_CHUNK_SIZE: u64 = 64 * 1024 * 1024;

/// A slow upload link for the token-lifetime estimate.
const PESSIMISTIC_UPLOAD_BYTES_PER_SECOND: u64 = 1024 * 1024;
/// Added to the estimate for the requests around the upload itself.
const UPLOAD_ESTIMATE_SLACK: Duration = Duration::minutes(15);

/// Key id of the placeholder credentials served after a failed renewal.
const FAILED_RENEWAL_KEY_ID: &str = "FLOXRENEWALFAILED";

const AWS_CONFIG_FILE_NAME: &str = "aws-config";
const CACHE_FILE_NAME: &str = "credentials.json";
const RENEWAL_FILE_NAME: &str = "renewal.json";
const RENEWAL_ERROR_FILE_NAME: &str = "renewal-error";

/// Multipart query parameters for an ingress store URL.
///
/// Only `s3://` stores upload in parts, and only Nix 2.33+, whose S3 store
/// uploads through curl, can combine multipart with the `.ls` content
/// encoding; Nix 2.31 refuses that combination outright.
pub fn multipart_params(url: &Url, nix_version: &str) -> Vec<(&'static str, String)> {
    if url.scheme() != "s3" || !supports_curl_s3_multipart(nix_version) {
        return vec![];
    }
    vec![
        ("multipart-upload", "true".to_string()),
        ("multipart-chunk-size", MULTIPART_CHUNK_SIZE.to_string()),
    ]
}

fn supports_curl_s3_multipart(nix_version: &str) -> bool {
    let mut parts = nix_version.split('.').map(str::parse::<u32>);
    match (parts.next(), parts.next()) {
        (Some(Ok(major)), Some(Ok(minor))) => (major, minor) >= (2, 33),
        _ => false,
    }
}

/// The version of the Nix `nix copy` will run, from `nix --version`.
///
/// Asked at runtime because `NIX_BIN` may point at a different Nix than the
/// one flox was built with.
pub fn running_nix_version() -> Option<String> {
    let output = nix_base_command().arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    // "nix (Nix) 2.35.3"
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout.split_whitespace().last()?.to_string();
    debug!(%version, "running nix version");
    Some(version)
}

/// How the helper authenticates to FloxHub when it renews.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalCredential {
    /// A FloxHub token, as `flox publish` used it.
    Token(String),
    /// A Kerberos ticket from the user's credential cache.
    Kerberos,
    /// No credential.
    None,
}

/// What `flox publish` knows about renewing upload credentials.
#[derive(Clone, Debug, PartialEq)]
pub struct UploadRenewal {
    /// The `flox` binary that serves `publish-credentials`.
    pub flox_bin: PathBuf,
    pub floxhub_api_url: String,
    pub credential: RenewalCredential,
    /// When the FloxHub credential expires, if known locally.
    pub token_expires_at: Option<DateTime<Utc>>,
    /// Where the per-publish credential directory is created.
    pub temp_dir: PathBuf,
}

/// Renewal of one upload session, for one catalog and package.
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRenewal {
    pub renewal: UploadRenewal,
    pub catalog: String,
    pub package: String,
    /// When FloxHub handed out the session being renewed.
    pub issued_at: DateTime<Utc>,
}

/// What the helper needs to ask FloxHub for a fresh session.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RenewalRequest {
    pub catalog: String,
    pub package: String,
    pub floxhub_api_url: String,
    pub credential: RenewalCredential,
}

/// The `credential_process` output format.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ProcessCredentials {
    pub version: u8,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub expiration: DateTime<Utc>,
}

impl ProcessCredentials {
    /// Read a FloxHub `aws-s3` session, received at `issued_at`.
    fn from_catalog_auth(
        auth: &CatalogAuth,
        issued_at: DateTime<Utc>,
    ) -> Result<(Self, Option<String>), String> {
        let envs = catalog_auth_to_envs(auth).map_err(|e| e.to_string())?;
        let get = |name: &str| {
            envs.get(name)
                .cloned()
                .ok_or_else(|| format!("FloxHub sent upload credentials without {name}"))
        };
        let expiration = match envs.get(EXPIRATION_ENV) {
            Some(value) => DateTime::parse_from_rfc3339(value)
                .map_err(|e| format!("invalid {EXPIRATION_ENV} '{value}': {e}"))?
                .with_timezone(&Utc),
            None => issued_at + ASSUMED_SESSION_LIFETIME,
        };
        let credentials = Self {
            version: 1,
            access_key_id: get("AWS_ACCESS_KEY_ID")?,
            secret_access_key: get("AWS_SECRET_ACCESS_KEY")?,
            session_token: get("AWS_SESSION_TOKEN")?,
            expiration: expiration.trunc_subsecs(0),
        };
        Ok((credentials, envs.get("AWS_REGION").cloned()))
    }

    /// Credentials S3 rejects, served after a failed renewal so that Nix's
    /// credential chain stops at the profile instead of trying EC2 metadata.
    fn placeholder(now: DateTime<Utc>) -> Self {
        Self {
            version: 1,
            access_key_id: FAILED_RENEWAL_KEY_ID.to_string(),
            secret_access_key: "renewal-failed".to_string(),
            session_token: "renewal-failed".to_string(),
            expiration: now.trunc_subsecs(0),
        }
    }
}

/// The per-publish directory holding the AWS profile and the credential
/// cache. Removed when dropped.
#[derive(Debug)]
pub struct UploadCredentials {
    dir: TempDir,
    region: Option<String>,
}

impl UploadCredentials {
    /// Write the AWS profile, the first credential set and what the helper
    /// needs to renew it, all private to the user, in a new directory.
    pub fn prepare(initial: &CatalogAuth, session: &SessionRenewal) -> Result<Self, PublishError> {
        let SessionRenewal {
            renewal,
            catalog,
            package,
            issued_at,
        } = session;
        let io_err = |e: std::io::Error| {
            PublishError::Catchall(format!("failed to prepare upload credentials: {e}"))
        };
        let (credentials, region) = ProcessCredentials::from_catalog_auth(initial, *issued_at)
            .map_err(PublishError::Catchall)?;
        let dir = tempfile::Builder::new()
            .prefix("upload-credentials-")
            .tempdir_in(&renewal.temp_dir)
            .map_err(io_err)?;

        let command = format!(
            "{} publish-credentials --dir {}",
            shell_escape::unix::escape(renewal.flox_bin.to_string_lossy()),
            shell_escape::unix::escape(dir.path().to_string_lossy()),
        );
        let config = format!("[profile {PROFILE}]\ncredential_process = {command}\n");
        write_private(&dir.path().join(AWS_CONFIG_FILE_NAME), config.as_bytes()).map_err(io_err)?;
        write_json(&dir.path().join(CACHE_FILE_NAME), &credentials).map_err(io_err)?;
        write_json(&dir.path().join(RENEWAL_FILE_NAME), &RenewalRequest {
            catalog: catalog.clone(),
            package: package.clone(),
            floxhub_api_url: renewal.floxhub_api_url.clone(),
            credential: renewal.credential.clone(),
        })
        .map_err(io_err)?;

        Ok(Self { dir, region })
    }

    /// The directory the helper serves from.
    pub fn dir(&self) -> &Path {
        self.dir.path()
    }

    /// The AWS profile to name in the store URL.
    pub fn profile(&self) -> &str {
        PROFILE
    }

    /// Point `cmd` at the per-publish profile, and away from any AWS
    /// identity the user has: every inherited `AWS_*` variable is removed.
    pub fn apply(&self, cmd: &mut Command) {
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("AWS_") {
                cmd.env_remove(name);
            }
        }
        cmd.env("AWS_CONFIG_FILE", self.dir().join(AWS_CONFIG_FILE_NAME));
        cmd.env("AWS_SHARED_CREDENTIALS_FILE", "/dev/null");
        if let Some(region) = &self.region {
            cmd.env("AWS_REGION", region);
        }
    }

    /// The reason the helper last failed to renew, if it did.
    pub fn renewal_failure(&self) -> Option<String> {
        std::fs::read_to_string(self.dir().join(RENEWAL_ERROR_FILE_NAME)).ok()
    }
}

/// Answer one `credential_process` call from `dir`: the cached set while
/// more than [RENEWAL_MARGIN] of it remains, otherwise a fresh set from
/// `renew`, which is cached for the calls after it.
///
/// On failure the reason is recorded for `flox publish` and placeholder
/// credentials are returned; see the module docs. The upload has failed by
/// then, so later calls return the placeholder without asking FloxHub again.
pub async fn serve(
    dir: &Path,
    now: DateTime<Utc>,
    renew: impl AsyncFnOnce(&RenewalRequest) -> Result<CatalogAuth, String>,
) -> ProcessCredentials {
    let cache_path = dir.join(CACHE_FILE_NAME);
    if let Some(cached) = read_json::<ProcessCredentials>(&cache_path)
        && cached.expiration - now > RENEWAL_MARGIN
    {
        return cached;
    }

    let error_path = dir.join(RENEWAL_ERROR_FILE_NAME);
    if error_path.exists() {
        return ProcessCredentials::placeholder(now);
    }

    let result = async {
        let request = read_json::<RenewalRequest>(&dir.join(RENEWAL_FILE_NAME))
            .ok_or_else(|| format!("no publish to renew credentials for in {}", dir.display()))?;
        let auth = renew(&request).await?;
        let (credentials, _region) = ProcessCredentials::from_catalog_auth(&auth, now)?;
        write_json(&cache_path, &credentials)
            .map_err(|e| format!("failed to cache upload credentials: {e}"))?;
        Ok::<_, String>(credentials)
    }
    .await;

    match result {
        Ok(credentials) => credentials,
        Err(reason) => {
            debug!(%reason, "failed to renew upload credentials");
            let _ = write_private(&error_path, reason.as_bytes());
            ProcessCredentials::placeholder(now)
        },
    }
}

/// Refuse an upload the FloxHub credential will not outlive, since renewing
/// upload credentials needs it until the last part.
///
/// The estimate assumes a slow link, so it errs towards asking for a new
/// login rather than failing hours into an upload.
pub fn check_token_outlives_upload(
    token_expires_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    upload_bytes: u64,
) -> Result<(), PublishError> {
    let Some(expires_at) = token_expires_at else {
        return Ok(());
    };
    let seconds = upload_bytes / PESSIMISTIC_UPLOAD_BYTES_PER_SECOND;
    let estimate = Duration::seconds(i64::try_from(seconds).unwrap_or(i64::MAX / 1000))
        + UPLOAD_ESTIMATE_SLACK;
    if now + estimate <= expires_at {
        return Ok(());
    }
    Err(PublishError::TokenExpiresDuringUpload {
        expires_in_minutes: (expires_at - now).num_minutes().max(0),
        upload_minutes: estimate.num_minutes(),
    })
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let contents = std::fs::read(path).ok()?;
    serde_json::from_slice(&contents).ok()
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    write_private(path, &serde_json::to_vec(value)?)
}

/// Replace `path` atomically with a file only the user can read; the helper
/// may run concurrently for parallel requests.
fn write_private(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    // NamedTempFile is created with mode 0600.
    let mut file = NamedTempFile::new_in(parent)?;
    file.write_all(contents)?;
    file.flush()?;
    file.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// The environment `cmd` will set or remove, for tests.
#[cfg(test)]
fn command_envs(cmd: &Command) -> std::collections::BTreeMap<String, Option<String>> {
    cmd.get_envs()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.map(|v| v.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    use pretty_assertions::assert_eq;
    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn session(key: &str) -> CatalogAuth {
        json!({
            "aws-s3": {
                "envs": {
                    "AWS_ACCESS_KEY_ID": key,
                    "AWS_SECRET_ACCESS_KEY": format!("{key}-secret"),
                    "AWS_SESSION_TOKEN": format!("{key}-token"),
                    "AWS_REGION": "us-east-1",
                }
            }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn session_renewal(parent: &Path) -> SessionRenewal {
        SessionRenewal {
            renewal: UploadRenewal {
                flox_bin: PathBuf::from("/opt/flox dir/bin/flox"),
                floxhub_api_url: "https://api.flox.dev".to_string(),
                credential: RenewalCredential::Token("floxhub-token".to_string()),
                token_expires_at: None,
                temp_dir: parent.to_path_buf(),
            },
            catalog: "myorg".to_string(),
            package: "model".to_string(),
            issued_at: t0(),
        }
    }

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-11-26T20:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn prepare(parent: &Path) -> UploadCredentials {
        UploadCredentials::prepare(&session("FIRST"), &session_renewal(parent)).unwrap()
    }

    fn credentials(key: &str, expiration: DateTime<Utc>) -> ProcessCredentials {
        ProcessCredentials {
            version: 1,
            access_key_id: key.to_string(),
            secret_access_key: format!("{key}-secret"),
            session_token: format!("{key}-token"),
            expiration,
        }
    }

    #[test]
    fn multipart_params_only_for_s3() {
        let s3 = Url::parse("s3://bucket/prefix?region=us-east-1").unwrap();
        let https = Url::parse("https://cache.example.com").unwrap();
        let file = Url::parse("file:///tmp/cache").unwrap();
        assert_eq!(
            (
                multipart_params(&s3, "2.35.3"),
                multipart_params(&https, "2.35.3"),
                multipart_params(&file, "2.35.3"),
            ),
            (
                vec![
                    ("multipart-upload", "true".to_string()),
                    ("multipart-chunk-size", "67108864".to_string()),
                ],
                vec![],
                vec![],
            )
        );
    }

    #[test]
    fn multipart_params_need_curl_s3_nix() {
        let s3 = Url::parse("s3://bucket").unwrap();
        let versions = ["2.31.6", "2.32.0", "2.33.0", "2.35.3", "3.0.0", "unknown"];
        let enabled = versions
            .iter()
            .map(|version| !multipart_params(&s3, version).is_empty())
            .collect::<Vec<_>>();
        assert_eq!(enabled, [false, false, true, true, true, false]);
    }

    #[test]
    fn prepare_writes_private_config() {
        let parent = tempdir().unwrap();
        let creds = prepare(parent.path());
        let config_path = creds.dir().join(AWS_CONFIG_FILE_NAME);
        let modes = [AWS_CONFIG_FILE_NAME, CACHE_FILE_NAME, RENEWAL_FILE_NAME].map(|name| {
            std::fs::metadata(creds.dir().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        });
        assert_eq!(
            (std::fs::read_to_string(config_path).unwrap(), modes),
            (
                format!(
                    "[profile flox-publish]\ncredential_process = '/opt/flox dir/bin/flox' publish-credentials --dir {}\n",
                    creds.dir().display()
                ),
                [0o600; 3],
            )
        );
    }

    #[test]
    fn nix_copy_env_has_no_user_aws() {
        let parent = tempdir().unwrap();
        let creds = prepare(parent.path());
        // Unset whatever AWS_* the developer running the test has, then set
        // a user's identity.
        let mut vars: Vec<(String, Option<String>)> = std::env::vars()
            .filter(|(name, _)| name.starts_with("AWS_"))
            .map(|(name, _)| (name, None))
            .collect();
        vars.extend([
            ("AWS_PROFILE".to_string(), Some("personal".to_string())),
            (
                "AWS_ACCESS_KEY_ID".to_string(),
                Some("AKIAUSER".to_string()),
            ),
            (
                "AWS_CONFIG_FILE".to_string(),
                Some("/home/u/.aws/config".to_string()),
            ),
        ]);
        let envs = temp_env::with_vars(vars, || {
            let mut cmd = Command::new("nix");
            creds.apply(&mut cmd);
            command_envs(&cmd)
        });
        assert_eq!(
            envs,
            BTreeMap::from([
                ("AWS_ACCESS_KEY_ID".to_string(), None),
                (
                    "AWS_CONFIG_FILE".to_string(),
                    Some(creds.dir().join("aws-config").display().to_string())
                ),
                ("AWS_PROFILE".to_string(), None),
                ("AWS_REGION".to_string(), Some("us-east-1".to_string())),
                (
                    "AWS_SHARED_CREDENTIALS_FILE".to_string(),
                    Some("/dev/null".to_string())
                ),
            ])
        );
    }

    #[tokio::test]
    async fn helper_serves_cached_until_margin() {
        let parent = tempdir().unwrap();
        let creds = prepare(parent.path());
        // 15-minute session from t0; still 5 min 1 s left at t0 + 9:59.
        let now = t0() + Duration::seconds(599);
        let served = serve(creds.dir(), now, async |_| {
            Err("must not renew while the cache is fresh".to_string())
        })
        .await;
        assert_eq!(
            (served, creds.renewal_failure()),
            (credentials("FIRST", t0() + Duration::minutes(15)), None)
        );
    }

    #[tokio::test]
    async fn helper_renews_under_margin() {
        let parent = tempdir().unwrap();
        let creds = prepare(parent.path());
        let now = t0() + Duration::minutes(11);
        let mut asked = None;
        let served = serve(creds.dir(), now, async |request| {
            asked = Some(request.clone());
            Ok(session("SECOND"))
        })
        .await;
        // The next call is served from the renewed cache.
        let later = serve(creds.dir(), now + Duration::minutes(1), async |_| {
            Err("must not renew again".to_string())
        })
        .await;
        let expected = credentials("SECOND", now + Duration::minutes(15));
        assert_eq!(
            (served, later, asked),
            (
                expected.clone(),
                expected,
                Some(RenewalRequest {
                    catalog: "myorg".to_string(),
                    package: "model".to_string(),
                    floxhub_api_url: "https://api.flox.dev".to_string(),
                    credential: RenewalCredential::Token("floxhub-token".to_string()),
                })
            )
        );
    }

    #[tokio::test]
    async fn helper_renew_failure_is_reported() {
        let parent = tempdir().unwrap();
        let creds = prepare(parent.path());
        let now = t0() + Duration::minutes(11);
        let served = serve(creds.dir(), now, async |_| {
            Err("FloxHub returned 503 Service Unavailable".to_string())
        })
        .await;
        // Later calls fail without asking FloxHub again.
        let again = serve(creds.dir(), now + Duration::seconds(1), async |_| {
            Ok(session("MUST-NOT-BE-ASKED"))
        })
        .await;
        let reported = creds
            .renewal_failure()
            .map(|reason| PublishError::RenewUploadCredentials(reason).to_string());
        assert_eq!(
            (served, again, reported),
            (
                ProcessCredentials::placeholder(now),
                ProcessCredentials::placeholder(now + Duration::seconds(1)),
                Some(indoc::indoc! {"
                    Failed to upload to cache: could not renew upload credentials: FloxHub returned 503 Service Unavailable
                    Run 'flox publish' again to start the upload over."}
                .to_string())
            )
        );
    }

    #[test]
    fn helper_output_is_credential_process_json() {
        let json = serde_json::to_value(credentials("KEY", t0())).unwrap();
        assert_eq!(
            json,
            json!({
                "Version": 1,
                "AccessKeyId": "KEY",
                "SecretAccessKey": "KEY-secret",
                "SessionToken": "KEY-token",
                "Expiration": "2026-11-26T20:00:00Z",
            })
        );
    }

    #[test]
    fn publish_refuses_token_expiring_before_upload() {
        let fifteen_gb = 15 * 1000 * 1000 * 1000;
        let expires_at = t0() + Duration::hours(2);
        let result = check_token_outlives_upload(Some(expires_at), t0(), fifteen_gb);
        assert_eq!(
            result.map_err(|e| e.to_string()),
            Err(indoc::indoc! {"
                FloxHub login expires in 120 minutes, before this upload may finish.
                On a slow connection the upload may take up to 253 minutes.
                Run 'flox auth login' to start a new login, then publish again."}
            .to_string())
        );
    }

    #[test]
    fn token_outliving_upload_is_accepted() {
        let results = [
            check_token_outlives_upload(None, t0(), u64::MAX).is_ok(),
            check_token_outlives_upload(Some(t0() + Duration::hours(2)), t0(), 100 << 20).is_ok(),
        ];
        assert_eq!(results, [true, true]);
    }
}
