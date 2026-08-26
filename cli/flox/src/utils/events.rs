//! Integration wrapper between the flox binary and the [`flox_events`] crate.
//!
//! The CLI emits two telemetry streams in parallel: the legacy
//! `subcommand_metric!` pipeline (`cli/flox/src/utils/metrics.rs`) and the
//! v2-events pipeline (this module + the [`flox_events`] crate). The two
//! stacks share no code and write separate on-disk buffers.
//! `config.flox.disable_metrics` silences both.
//!
//! Authenticated invocations additionally carry a pseudonymous subject
//! identifier as `auth_subject`. This wrapper derives it and `credential_type`
//! from the invocation's `AuthContext`; the events client retains only those
//! sanitized values.

use std::env;
use std::str::FromStr;
use std::sync::{LazyLock, OnceLock};

use flox_config::Config;
use flox_events::{CredentialType, EnvDetail, EventsClient, EventsHub, SharedMetadataTemplate};
use flox_rust_sdk::flox::{AuthContext, FLOX_VERSION, Flox};
use flox_rust_sdk::models::environment::generations::GenerationsExt;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::utils::INVOCATION_SOURCES;
use tracing::debug;
use uuid::Uuid;

use crate::utils::detect_shell::detect_shell_name_for_metrics;
use crate::utils::local_environment_id;
use crate::utils::metrics::read_metrics_uuid;

/// Stores the invocation_id resolved by [`resolve_invocation_id`] so detached
/// subprocess spawn sites can propagate it via [`FLOX_INVOCATION_ID_VAR`].
///
/// Kept out of the process environment so that an activated user shell does
/// not inherit it — `flox` commands run from inside an activated shell are
/// fresh top-level invocations.
static RESOLVED_INVOCATION_ID: OnceLock<Uuid> = OnceLock::new();

/// Env var carrying the parent flox process's invocation id across a
/// detached subprocess boundary.
pub const FLOX_INVOCATION_ID_VAR: &str = "FLOX_INVOCATION_ID";

static METRICS_EVENTS_URL_V2: LazyLock<String> = LazyLock::new(|| {
    std::env::var("_FLOX_METRICS_URL_V2_OVERRIDE")
        .unwrap_or(env!("METRICS_EVENTS_URL_V2").to_string())
});
static METRICS_EVENTS_API_KEY_V2: LazyLock<String> = LazyLock::new(|| {
    std::env::var("_FLOX_METRICS_API_KEY_V2_OVERRIDE")
        .unwrap_or(env!("METRICS_EVENTS_API_KEY_V2").to_string())
});

/// Resolve the invocation id for the current process.
///
/// If [`FLOX_INVOCATION_ID_VAR`] is set and parses as a UUID, the process
/// inherits it from a parent flox invocation so its v2 events join the
/// parent's stream. Otherwise a fresh v4 UUID is minted, marking this as a
/// top-level invocation.
pub fn resolve_invocation_id() -> Uuid {
    let resolved = match env::var(FLOX_INVOCATION_ID_VAR) {
        Ok(raw) => match Uuid::from_str(&raw) {
            Ok(uuid) => {
                debug!(invocation_id = %uuid, "inherited v2 invocation_id from FLOX_INVOCATION_ID");
                uuid
            },
            Err(err) => {
                debug!(error = %err, "FLOX_INVOCATION_ID set but unparseable; minting fresh id");
                Uuid::new_v4()
            },
        },
        Err(_) => Uuid::new_v4(),
    };
    let _ = RESOLVED_INVOCATION_ID.set(resolved);
    resolved
}

/// Return the invocation_id resolved by [`resolve_invocation_id`] earlier in
/// this process, if any. Detached subprocess spawn sites use this to set
/// [`FLOX_INVOCATION_ID_VAR`] on the child's `Command`.
pub fn current_invocation_id() -> Option<Uuid> {
    RESOLVED_INVOCATION_ID.get().copied()
}

/// Saturate a duration into whole ms (u64::MAX ms is ~584M years — a
/// ceiling only).
pub(crate) fn duration_to_ms(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
}

fn credential_type_from_context(auth_context: &AuthContext) -> CredentialType {
    match auth_context {
        AuthContext::Auth0(Some(_)) | AuthContext::Bare(_) => CredentialType::OAuthAccessToken,
        AuthContext::AccessToken(token) if token.secret().starts_with("flox_pat_") => {
            CredentialType::Pat
        },
        AuthContext::AccessToken(token) if token.secret().starts_with("flox_sat_") => {
            CredentialType::ServiceToken
        },
        AuthContext::AccessToken(_) => CredentialType::OAuthAccessToken,
        AuthContext::Auth0(None) | AuthContext::Kerberos(_) => CredentialType::None,
    }
}

/// Build the [`SharedMetadataTemplate`] stamped onto every v2 event emitted
/// by this process. The fields mirror the legacy
/// [`crate::utils::metrics::MetricEntry`] so downstream consumers can
/// reconstruct the existing columns.
fn shared_metadata_template(credential_type: CredentialType) -> SharedMetadataTemplate {
    let linux_release = sys_info::linux_os_release().ok();
    SharedMetadataTemplate {
        credential_type,
        flox_version: FLOX_VERSION.to_string(),
        os_family: sys_info::os_type()
            .ok()
            .map(|x| x.replace("Darwin", "Mac OS")),
        os_family_release: sys_info::os_release().ok(),
        os: linux_release.as_ref().and_then(|r| r.id.clone()),
        os_version: linux_release.and_then(|r| r.version_id),
        os_platform_version: macos_product_version(),
        shell: detect_shell_name_for_metrics().map(String::from),
        architecture: architecture_from_system(env!("NIX_TARGET_SYSTEM")),
        empty_flags: vec![],
        invocation_sources: INVOCATION_SOURCES.clone(),
    }
}

/// macOS product version (e.g. `15.5`) via [`sysinfo::System::os_version`]
/// — NOT the kernel release that the near-identically-named
/// `sys_info::os_release()` puts in `os_family_release`. Best-effort `None`.
fn macos_product_version() -> Option<String> {
    #[cfg(target_os = "macos")]
    let raw = sysinfo::System::os_version();
    #[cfg(not(target_os = "macos"))]
    let raw = None;
    normalize_macos_product_version(raw)
}

/// A binary linked against a pre-macOS-11 SDK (or run under the system's
/// version-compat shim) reads back a constant `10.16`. No real macOS ever
/// reported that — Big Sur is 11.x — so treat it as unknown, not a version.
fn normalize_macos_product_version(raw: Option<String>) -> Option<String> {
    raw.filter(|version| version != "10.16" && !version.starts_with("10.16."))
}

/// Arch component of a Nix system double (`aarch64-darwin` → `aarch64`).
/// Deliberately the build target, not a host-CPU probe — Rosetta semantics
/// are pinned on the wire field's doc.
fn architecture_from_system(system: &str) -> Option<String> {
    system
        .split_once('-')
        .map(|(architecture, _os)| architecture.to_string())
}

/// Try to build an [`EventsClient`] to install on the global
/// [`flox_events::EventsHub`].
///
/// Clients across invocations share an anonymous per-installation id via
/// [`read_metrics_uuid`].
///
/// Returns `None` if
/// a) metrics are disabled by config, or
/// b) reading the metrics uuid fails.
///
/// `auth_context` is the credential selected for the invocation. The
/// returned client snapshots its pseudonymous subject and local credential
/// kind at construction (see [`flox_events::EventsClient`] for the snapshot
/// semantics); credential material is never retained by the events client.
///
/// Ordering invariant: [`shared_metadata_template`] performs machine-context
/// detection, so it must stay behind the `disable_metrics` early return —
/// opted-out runs do no metrics-only metadata work.
pub fn build_events_client(
    config: &Config,
    invocation_id: Uuid,
    auth_context: &AuthContext,
) -> Option<EventsClient> {
    if config.flox.disable_metrics {
        debug!("v2 events: disable_metrics is true; not installing client");
        return None;
    }

    let device_id = match read_metrics_uuid(config) {
        Ok(uuid) => uuid,
        Err(err) => {
            debug!(error = %err, "v2 events: could not read metrics uuid; not installing client");
            return None;
        },
    };

    let auth_subject = auth_context.user_subject().map(String::from);
    let credential_type = credential_type_from_context(auth_context);
    Some(EventsClient::new(
        device_id,
        &config.flox.data_dir,
        METRICS_EVENTS_URL_V2.clone(),
        METRICS_EVENTS_API_KEY_V2.clone(),
        invocation_id,
        auth_subject,
        shared_metadata_template(credential_type),
    ))
}

/// Lineage fields for the environment the current invocation operates on.
struct EnvLineageFields {
    local_environment_id: Option<Uuid>,
    generation_number: Option<u64>,
    package_count: Option<u64>,
}

/// Read the lineage fields for `env`. Every read is best-effort and read-only:
/// a failure leaves the field absent and is logged at debug, never failing the
/// command.
fn read_env_lineage_fields(flox: &Flox, env: &ConcreteEnvironment) -> EnvLineageFields {
    let local_environment_id = match env {
        ConcreteEnvironment::Path(environment) => {
            local_environment_id::read(&environment.dot_flox_path())
        },
        // The CLI has no server-assigned id for managed/remote environments.
        // Their events carry owner/name in `env_ref_or_name`, so they emit no
        // local id.
        ConcreteEnvironment::Managed(_) | ConcreteEnvironment::Remote(_) => None,
    };

    // Managed/remote resolve their generation and lockfile in a single
    // metadata read; path envs have no generation and read their on-disk
    // lockfile.
    let (generation, lockfile) = match env {
        ConcreteEnvironment::Path(environment) => (None, environment.existing_lockfile(flox)),
        ConcreteEnvironment::Managed(environment) => {
            match environment.generation_and_existing_lockfile() {
                Ok((generation, lockfile)) => (generation, Ok(lockfile)),
                Err(err) => (None, Err(err)),
            }
        },
        ConcreteEnvironment::Remote(environment) => {
            match environment.generation_and_existing_lockfile() {
                Ok((generation, lockfile)) => (generation, Ok(lockfile)),
                Err(err) => (None, Err(err)),
            }
        },
    };

    let generation_number = generation.map(|generation| *generation as u64);

    let package_count = lockfile
        .map_err(|err| debug!(error = %err, "v2 events: could not read lockfile"))
        .ok()
        .flatten()
        .map(|lockfile| {
            lockfile
                .packages
                .iter()
                .filter(|package| package.system().as_str() == flox.system.as_str())
                .count() as u64
        });

    EnvLineageFields {
        local_environment_id,
        generation_number,
        package_count,
    }
}

/// Build an [`EnvDetail`] for the supplied [`ConcreteEnvironment`], using the
/// same env-kind / env-ref mapping as the legacy
/// `environment_subcommand_metric!` macro. Shared across call sites so the
/// per-kind match is not duplicated.
fn env_detail_with_lineage(
    env: &ConcreteEnvironment,
    generation_number: Option<u64>,
    local_environment_id: Option<Uuid>,
) -> EnvDetail {
    match env {
        ConcreteEnvironment::Remote(environment) => {
            EnvDetail::remote(environment.env_ref().to_string(), generation_number)
        },
        ConcreteEnvironment::Managed(environment) => {
            EnvDetail::managed(environment.env_ref().to_string(), generation_number)
        },
        ConcreteEnvironment::Path(environment) => EnvDetail::path(
            Environment::name(environment).to_string(),
            local_environment_id,
        ),
    }
}

/// Build environment identity without reading lineage. Used by eager events
/// that must emit before locking or trust decisions.
pub fn env_detail_from_concrete_without_lineage(env: &ConcreteEnvironment) -> EnvDetail {
    env_detail_with_lineage(env, None, None)
}

/// Build environment detail, reading lineage only when an events client is
/// installed.
pub fn env_detail_from_concrete(flox: &Flox, env: &ConcreteEnvironment) -> EnvDetail {
    let Some(fields) = EventsHub::global().when_client_set(|| read_env_lineage_fields(flox, env))
    else {
        return env_detail_from_concrete_without_lineage(env);
    };
    let mut detail =
        env_detail_with_lineage(env, fields.generation_number, fields.local_environment_id);
    if let Some(package_count) = fields.package_count {
        detail = detail.with_package_count(package_count);
    }
    detail
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use flox_config::FloxConfig;
    use flox_events::test_helpers::MockEventsConnection;
    use flox_events::{EVENTS_BUFFER_FILE_NAME, EventsHub, LifecycleFields};
    use floxhub_client::test_helpers::{FAKE_TOKEN, FAKE_TOKEN_NO_HANDLE, FAKE_TOKEN_WITH_SUB};
    use serial_test::serial;
    use temp_env::with_var;
    use tempfile::TempDir;

    use super::*;

    /// A `Config` value pointing at a fresh tempdir, with metrics enabled
    /// and a pre-written metrics uuid so the wrapper has everything it
    /// needs to install a client.
    fn test_config_with_uuid(tempdir: &TempDir, uuid: Uuid) -> Config {
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        std::fs::write(data_dir.join("metrics-uuid"), uuid.hyphenated().to_string())
            .expect("write metrics-uuid");
        test_config(tempdir, data_dir, /* disable_metrics */ false)
    }

    #[allow(deprecated)]
    fn test_config(tempdir: &TempDir, data_dir: PathBuf, disable_metrics: bool) -> Config {
        Config {
            flox: FloxConfig {
                cache_dir: tempdir.path().join("cache"),
                data_dir,
                state_dir: tempdir.path().join("state"),
                config_dir: tempdir.path().join("config"),
                disable_metrics,
                ..FloxConfig::default()
            },
            features: None,
        }
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_returns_parent_id_when_env_set() {
        let parent = Uuid::new_v4();
        with_var(FLOX_INVOCATION_ID_VAR, Some(parent.to_string()), || {
            assert_eq!(resolve_invocation_id(), parent);
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unset() {
        with_var(FLOX_INVOCATION_ID_VAR, None::<&str>, || {
            let a = resolve_invocation_id();
            let b = resolve_invocation_id();
            assert_ne!(a, Uuid::nil());
            assert_ne!(a, b, "consecutive mints should not collide");
        });
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn resolve_invocation_id_mints_fresh_when_env_unparseable() {
        with_var(FLOX_INVOCATION_ID_VAR, Some("not-a-uuid"), || {
            let id = resolve_invocation_id();
            assert_ne!(id, Uuid::nil());
        });
    }

    #[test]
    fn normalize_macos_product_version_drops_compat_shim_sentinel() {
        // `10.16` is the version-compat shim's constant, never a real
        // product version — it must read as unknown, not ship on the wire.
        assert_eq!(
            normalize_macos_product_version(Some("10.16".to_string())),
            None
        );
        assert_eq!(
            normalize_macos_product_version(Some("10.16.0".to_string())),
            None
        );
        assert_eq!(
            normalize_macos_product_version(Some("15.5".to_string())).as_deref(),
            Some("15.5")
        );
        assert_eq!(normalize_macos_product_version(None), None);
    }

    #[test]
    fn architecture_from_system_extracts_arch() {
        assert_eq!(
            architecture_from_system("aarch64-darwin").as_deref(),
            Some("aarch64")
        );
        assert_eq!(
            architecture_from_system("x86_64-linux").as_deref(),
            Some("x86_64")
        );
    }

    #[test]
    fn shared_metadata_template_populates_machine_context() {
        let template = shared_metadata_template(CredentialType::OAuthAccessToken);

        // Whole-struct compare: machine-dependent fields are cloned from the
        // actual value, architecture is pinned from the compile-time target,
        // and a new template field breaks this literal. Shell's chain
        // behavior is pinned in the detect_shell tests; the value-domain
        // assertion below guards the wire here.
        let expected = SharedMetadataTemplate {
            credential_type: CredentialType::OAuthAccessToken,
            flox_version: FLOX_VERSION.to_string(),
            os_family: template.os_family.clone(),
            os_family_release: template.os_family_release.clone(),
            os: template.os.clone(),
            os_version: template.os_version.clone(),
            os_platform_version: template.os_platform_version.clone(),
            shell: template.shell.clone(),
            architecture: architecture_from_system(env!("NIX_TARGET_SYSTEM")),
            empty_flags: vec![],
            invocation_sources: INVOCATION_SOURCES.clone(),
        };
        assert_eq!(template, expected);

        // A normalized name from the closed set — never a filesystem path
        // (which would carry usernames onto the wire).
        if let Some(shell) = template.shell.as_deref() {
            assert!(
                matches!(shell, "bash" | "zsh" | "fish" | "tcsh"),
                "shell must be a normalized supported-shell name, got {shell:?}"
            );
        }

        // architecture is the compile-time native target — always known.
        let arch = template
            .architecture
            .as_deref()
            .expect("architecture is compile-time known");
        assert!(matches!(arch, "aarch64" | "x86_64"));

        // os_platform_version is the macOS *product* version — macOS-only,
        // best-effort (the sysctl read may be denied in a sandbox), and
        // never the Darwin kernel release when it is present.
        if cfg!(target_os = "macos") {
            if let Some(product) = template.os_platform_version.as_deref() {
                assert!(product.chars().next().is_some_and(|c| c.is_ascii_digit()));
                assert_ne!(
                    template.os_platform_version, template.os_family_release,
                    "product version must stay distinct from the kernel release"
                );
            }
        } else {
            assert_eq!(
                template.os_platform_version, None,
                "os_platform_version is macOS-only"
            );
        }
    }

    #[test]
    fn credential_type_reflects_local_auth_context() {
        let contexts = [
            AuthContext::new_from_token(Some(FAKE_TOKEN)),
            AuthContext::new_from_token(Some(FAKE_TOKEN_NO_HANDLE)),
            AuthContext::new_from_token(Some("flox_pat_test")),
            AuthContext::new_from_token(Some("flox_sat_test")),
            AuthContext::new_from_token(Some("opaque-access-token")),
            AuthContext::new_from_token(Some("flox_unknown_test")),
            AuthContext::new_from_token(None),
            AuthContext::Kerberos(None),
        ];

        assert_eq!(
            contexts.map(|context| credential_type_from_context(&context)),
            [
                CredentialType::OAuthAccessToken,
                CredentialType::OAuthAccessToken,
                CredentialType::Pat,
                CredentialType::ServiceToken,
                CredentialType::OAuthAccessToken,
                CredentialType::OAuthAccessToken,
                CredentialType::None,
                CredentialType::None,
            ]
        );
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_disable_metrics_is_true() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = test_config(
            &tempdir,
            tempdir.path().join("data"),
            /* disable_metrics */ true,
        );

        let auth_context = AuthContext::new_from_token(None);
        let client = build_events_client(&config, Uuid::new_v4(), &auth_context);
        assert!(client.is_none(), "disable_metrics must take priority");
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_none_when_uuid_unreadable() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let data_dir = tempdir.path().join("data");
        std::fs::create_dir_all(&data_dir).expect("data dir");
        // No metrics-uuid file written: read_metrics_uuid errors.
        let config = test_config(&tempdir, data_dir, /* disable_metrics */ false);

        let auth_context = AuthContext::new_from_token(None);
        let client = build_events_client(&config, Uuid::new_v4(), &auth_context);
        assert!(client.is_none(), "missing uuid must short-circuit");
    }

    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_returns_some_by_default() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let uuid = Uuid::new_v4();
        let config = test_config_with_uuid(&tempdir, uuid);

        let auth_context = AuthContext::new_from_token(None);
        let client = build_events_client(&config, Uuid::new_v4(), &auth_context);
        assert!(client.is_some(), "v2 is enabled by default");
        assert_eq!(client.unwrap().device_id, uuid);
    }

    /// The wrapper derives the subject from the same auth context used for
    /// the credential type.
    #[test]
    #[serial(v2_events_wrapper_env)]
    fn build_events_client_derives_auth_subject_from_context() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let config = test_config_with_uuid(&tempdir, Uuid::new_v4());

        let authenticated = AuthContext::new_from_token(Some(FAKE_TOKEN_WITH_SUB));
        let client =
            build_events_client(&config, Uuid::new_v4(), &authenticated).expect("client installs");
        assert_eq!(client.auth_subject.as_deref(), Some("github|424242"));

        let unauthenticated = AuthContext::new_from_token(None);
        let client = build_events_client(&config, Uuid::new_v4(), &unauthenticated)
            .expect("client installs");
        assert_eq!(client.auth_subject, None, "anonymous use stays anonymous");
    }

    /// End-to-end: install a hub-owned client backed by a
    /// [`MockEventsConnection`], record run + completed for one invocation,
    /// and assert exactly one of each lands sharing one `invocation_id`.
    #[test]
    #[serial(global_events_client)]
    fn one_run_one_completed_end_to_end() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let invocation_id = Uuid::new_v4();

        let template = SharedMetadataTemplate {
            credential_type: CredentialType::None,
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: None,
            os: None,
            os_version: None,
            os_platform_version: None,
            shell: None,
            architecture: None,
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        };
        let client = EventsClient::new_with_connection(
            Uuid::new_v4(),
            tempdir.path(),
            invocation_id,
            None,
            template,
            connection,
        );

        let previous = EventsHub::global().set_client(client);

        EventsHub::global()
            .record_command_run("install".to_string())
            .expect("record run");
        EventsHub::global()
            .record_command_completed("install".to_string(), LifecycleFields {
                exit_code: 0,
                duration_ms: Some(1),
                error_kind: None,
            })
            .expect("record completed");
        EventsHub::global().flush(true).expect("flush");

        // Confirm only one buffer file was written and now drained.
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(EVENTS_BUFFER_FILE_NAME))
                .expect("read buffer"),
            ""
        );

        let batches = sent_batches.lock().unwrap().clone();
        let events: Vec<_> = batches.into_iter().flatten().collect();
        assert_eq!(events.len(), 2, "exactly one run + one completed");
        let invocations: Vec<_> = events.iter().map(|e| e.invocation_id).collect();
        assert!(
            invocations.iter().all(|id| *id == invocation_id),
            "events must share one invocation_id"
        );

        // Restore the previous client (which was None unless another test
        // installed one before us).
        EventsHub::global().clear_client();
        if let Some(previous) = previous {
            EventsHub::global().set_client(previous);
        }
    }
}
