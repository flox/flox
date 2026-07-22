//! V2 telemetry events emitted by the Flox CLI.
//!
//! This crate contains the v2 event envelope and the self-contained
//! pipeline for buffering and sending `cli.*` events. The global hub is dormant
//! until a client is installed by the CLI.

mod buffer;
mod client;
mod connection;
mod guard;
mod hub;

pub use buffer::{EVENTS_BUFFER_FILE_NAME, EventsBuffer};
pub use client::{BATCH_SIZE, EventsClient};
pub use connection::{EventsConnection, EventsConnectionV2, TRAILING_NETWORK_CALL_TIMEOUT};
pub use guard::{EventsGuard, force_flush_requested};
pub use hub::EventsHub;
use serde::{Deserialize, Serialize, de};
use serde_with::{TimestampMilliSeconds, serde_as};
use time::OffsetDateTime;
use uuid::Uuid;

#[cfg(any(test, feature = "tests"))]
pub mod test_helpers {
    pub use crate::connection::MockEventsConnection;
}

const CLI_SOURCE: &str = "cli";

/// A single telemetry event in the v2 envelope shape.
///
/// `source` is always `"cli"`. `kind` carries the variant tag and its
/// typed payload and is flattened into the envelope, so the wire shape
/// is `{event_id, event_timestamp, source, invocation_id, device_id,
/// auth_subject?, event_type, payload}`.
///
/// The CLI serializes events for transport and deserializes the same shape to
/// reload its local buffer.
#[serde_as]
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Event {
    /// Unique id for this event (used downstream for de-duplication).
    pub event_id: Uuid,
    /// When the event occurred. Serialized as an integer millisecond
    /// count since the Unix epoch — matches the downstream
    /// `DateTime64(3, 'UTC')` storage granularity, avoids the
    /// `f64`-mantissa precision loss that bites nanosecond timestamps
    /// when consumers parse JSON numbers as floats, and avoids the
    /// timezone-ambiguity class entirely (no offset, no DST gaps).
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    pub event_timestamp: OffsetDateTime,
    /// The producer. Always `"cli"`.
    pub source: &'static str,
    /// Correlates every event emitted during one CLI invocation.
    pub invocation_id: Uuid,
    /// Stable per-installation id.
    pub device_id: Uuid,
    /// Pseudonymous authenticated-subject identifier — the OIDC/JWT
    /// `sub` claim (sourced from the auth token) when known. Must not
    /// contain email addresses, raw user handles, or token bytes — those
    /// are PII and a different category from this field's pseudonymous-
    /// identifier contract.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_subject: Option<String>,
    /// The event variant and its typed payload. Flattened into the
    /// envelope: the variant's `#[serde(rename)]` becomes `event_type`
    /// and the variant's payload struct becomes `payload`.
    #[serde(flatten)]
    pub kind: EventKind,
}

#[serde_as]
#[derive(Debug, Deserialize)]
struct EventWire {
    event_id: Uuid,
    #[serde_as(as = "TimestampMilliSeconds<i64>")]
    event_timestamp: OffsetDateTime,
    source: String,
    invocation_id: Uuid,
    device_id: Uuid,
    auth_subject: Option<String>,
    #[serde(flatten)]
    kind: EventKind,
}

impl<'de> Deserialize<'de> for Event {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = EventWire::deserialize(deserializer)?;
        if wire.source != CLI_SOURCE {
            return Err(de::Error::custom(format!(
                "expected v2 event source {CLI_SOURCE:?}, got {:?}",
                wire.source
            )));
        }

        Ok(Self {
            event_id: wire.event_id,
            event_timestamp: wire.event_timestamp,
            source: CLI_SOURCE,
            invocation_id: wire.invocation_id,
            device_id: wire.device_id,
            auth_subject: wire.auth_subject,
            kind: wire.kind,
        })
    }
}

/// The set of event variants the CLI emits.
///
/// The dotted wire name on `#[serde(rename)]` is the single source of
/// truth for each variant; call sites construct the variant explicitly and
/// pass it to `record_event`, never a string literal. Variants share
/// payload types where the payloads are shape-identical — the variant is
/// the discriminant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "event_type", content = "payload")]
pub enum EventKind {
    #[serde(rename = "cli.command_run")]
    CliCommandRun(CliCommandRunPayload),
    #[serde(rename = "cli.command_completed")]
    CliCommandCompleted(CliCommandCompletedPayload),
    #[serde(rename = "cli.environment.activate")]
    CliEnvironmentActivate(CliEnvironmentActivatePayload),
    #[serde(rename = "cli.environment.push")]
    CliEnvironmentPush(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.pull")]
    CliEnvironmentPull(CliEnvironmentPayload),
    #[serde(rename = "cli.package.install")]
    CliPackageInstall(CliPackagePayload),
    #[serde(rename = "cli.package.upgrade")]
    CliPackageUpgrade(CliPackagePayload),
    #[serde(rename = "cli.package.uninstall")]
    CliPackageUninstall(CliPackagePayload),
    #[serde(rename = "cli.environment.containerize")]
    CliEnvironmentContainerize(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.delete")]
    CliEnvironmentDelete(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.edit")]
    CliEnvironmentEdit(CliEnvironmentEditPayload),
    #[serde(rename = "cli.environment.include.upgrade")]
    CliEnvironmentIncludeUpgrade(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.install")]
    CliEnvironmentInstall(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.list")]
    CliEnvironmentList(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.publish")]
    CliEnvironmentPublish(CliEnvironmentPublishPayload),
    #[serde(rename = "cli.environment.uninstall")]
    CliEnvironmentUninstall(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.upgrade")]
    CliEnvironmentUpgrade(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.start")]
    CliEnvironmentServicesStart(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.stop")]
    CliEnvironmentServicesStop(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.restart")]
    CliEnvironmentServicesRestart(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.status")]
    CliEnvironmentServicesStatus(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.logs")]
    CliEnvironmentServicesLogs(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.services.persist")]
    CliEnvironmentServicesPersist(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.generations.history")]
    CliEnvironmentGenerationsHistory(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.generations.list")]
    CliEnvironmentGenerationsList(CliEnvironmentGenerationsListPayload),
    #[serde(rename = "cli.environment.generations.rollback")]
    CliEnvironmentGenerationsRollback(CliEnvironmentPayload),
    #[serde(rename = "cli.environment.generations.switch")]
    CliEnvironmentGenerationsSwitch(CliEnvironmentPayload),
    #[serde(rename = "cli.build")]
    CliBuild(CliBuildPayload),
    #[serde(rename = "cli.search")]
    CliSearch(CliSearchPayload),
}

/// Shared metadata fields stamped onto every `cli.*` command event payload.
///
/// These fields drive existing `cli.telemetry` reporting downstream, so the
/// new pipeline carries them on its payloads to preserve continuity once the
/// cutover flips production traffic. The shape mirrors the columns the legacy
/// `MetricEntry` carries today (with `extras` deferred to per-domain payloads
/// in later PRs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandPayload {
    /// Subcommand name derived from the parsed bpaf command (e.g. `install`,
    /// `activate`, or nested `services::start` using the `parent::child`
    /// join encoding).
    subcommand: String,
    /// Flox CLI version string.
    flox_version: String,
    /// Coarse operating system family (e.g. `Mac OS`, `Linux`).
    os_family: Option<String>,
    /// OS family release version (the kernel version from
    /// `sys_info::os_release()`) — consumers derive every
    /// kernel-version-shaped legacy field from this one value.
    os_family_release: Option<String>,
    /// Linux distribution id (e.g. `ubuntu`); `None` outside Linux.
    os: Option<String>,
    /// Linux distribution version (e.g. `22.04`); `None` outside Linux.
    os_version: Option<String>,
    /// CLI flags that were observed empty on this invocation. Reserved for
    /// the per-command instrumentation PRs.
    empty_flags: Vec<String>,
    /// Tokens describing how this CLI invocation was launched (shell, prompt,
    /// service runner, etc.). Mirrors the legacy `INVOCATION_SOURCES`.
    /// CI / container membership is derived from these tokens by the
    /// consumer (a token equal to `"ci"` / `"containerd"` or a
    /// hierarchical sub-token like `"ci.github-actions"`, matched
    /// case-insensitively).
    invocation_sources: Vec<String>,
}

/// Static slice of [`CommandPayload`] that is constant for the duration of
/// one CLI invocation.
///
/// Pass into [`EventsClient::new`] so the client can stamp every command
/// event it emits without the call site rebuilding the same fields each
/// time. The `subcommand` field is supplied per-emission and merged in by
/// [`SharedMetadataTemplate::into_payload`].
#[derive(Debug, Clone)]
pub struct SharedMetadataTemplate {
    pub flox_version: String,
    pub os_family: Option<String>,
    pub os_family_release: Option<String>,
    pub os: Option<String>,
    pub os_version: Option<String>,
    pub empty_flags: Vec<String>,
    pub invocation_sources: Vec<String>,
}

impl SharedMetadataTemplate {
    /// Merge the stored static fields with the supplied subcommand to produce
    /// a complete [`CommandPayload`] ready for serialization.
    pub fn into_payload(&self, subcommand: String) -> CommandPayload {
        CommandPayload {
            subcommand,
            flox_version: self.flox_version.clone(),
            os_family: self.os_family.clone(),
            os_family_release: self.os_family_release.clone(),
            os: self.os.clone(),
            os_version: self.os_version.clone(),
            empty_flags: self.empty_flags.clone(),
            invocation_sources: self.invocation_sources.clone(),
        }
    }
}

/// Payload for [`EventKind::CliCommandRun`] — the event that carries the
/// full command context ([`CommandPayload`]). The other events of the same
/// invocation carry only their domain data; consumers join them to a run
/// row via `invocation_id`.
///
/// An `invocation_id` can carry more than one run row: detached background
/// children (e.g. the upgrade check spawned by `flox activate`) inherit the
/// parent's invocation id and emit their own `cli.command_run`. The
/// parent's run row is always the earliest-timestamped one for the
/// invocation — consumers joining for command context must use it.
///
/// The command-context join is best-effort under buffer overflow: the on-disk
/// buffer caps at 1000 events and evicts oldest-first, so a
/// `cli.command_completed` (or other domain) row can outlive the
/// `cli.command_run` it would join to. A single invocation pushes its run row
/// and its remaining events adjacently, so eviction drops whole invocations
/// rather than splitting a pair — but a consumer must tolerate a completed row
/// with no joinable run row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandRunPayload {
    #[serde(flatten)]
    command: CommandPayload,
}

impl CliCommandRunPayload {
    pub fn new(command: CommandPayload) -> Self {
        Self { command }
    }
}

/// The dispatch lifecycle stamped onto a `cli.command_completed` event,
/// serialized flattened into [`CliCommandCompletedPayload`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleFields {
    /// The exit code the invocation produces.
    pub exit_code: i32,
    /// Wall-clock duration in ms from dispatch start to completion (or to
    /// interrupt); `None` when the handler hands off instead of completing
    /// (e.g. the `activate` pre-`exec` emit), where no completion time is
    /// observable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// PII-safe slug for the failure, namespaced per error type (e.g.
    /// `environment.manifest_not_found`); `None` on success. Callers must
    /// derive it from a fixed set of compile-time strings (never from a
    /// rendered error) so user data cannot reach telemetry.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
}

/// Payload for [`EventKind::CliCommandCompleted`]. Carries the subcommand and
/// the dispatch lifecycle; the full command context lives on the invocation's
/// `cli.command_run` event, joinable by `invocation_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandCompletedPayload {
    subcommand: String,
    /// The dispatch lifecycle (exit code, duration, error kind). `Option` so
    /// the wire shape stays field-additive for any client that predates
    /// lifecycle reporting.
    #[serde(flatten)]
    lifecycle: Option<LifecycleFields>,
}

impl CliCommandCompletedPayload {
    pub fn new(subcommand: String, lifecycle: LifecycleFields) -> Self {
        Self {
            subcommand,
            lifecycle: Some(lifecycle),
        }
    }
}

/// Environment kind a `cli.environment.*` event touched, matching the three
/// legacy `environment_subcommand_metric!` arms (`remote_environment` /
/// `managed_environment` / `path_environment`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvDetail {
    /// One of `"remote"`, `"managed"`, `"path"` — the environment variant
    /// the command operated on. `"managed"` is also used for `flox pull`'s
    /// `NewAbbreviated` branch, where only the remote ref is known at
    /// emission time.
    env_kind: String,
    /// The environment's identifier — the result of `env_ref().to_string()`
    /// for remote and managed environments, and `Environment::name(...)`
    /// for path environments. Matches the value the legacy macros emit.
    env_ref_or_name: String,
    /// The environment's stable id from its `env.json`, when it has one.
    /// Absent for environments created before the id existed and for
    /// remote environments, whose cached pointer is rewritten per
    /// invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    environment_id: Option<Uuid>,
    /// Current generation the command started from. Absent for path
    /// environments, which have no generations.
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_number: Option<u64>,
    /// Number of packages locked for the invoking system (the `flox list`
    /// count) when the command started. Absent when no lockfile exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    package_count: Option<u64>,
}

impl EnvDetail {
    pub fn new(env_kind: impl Into<String>, env_ref_or_name: impl Into<String>) -> Self {
        Self {
            env_kind: env_kind.into(),
            env_ref_or_name: env_ref_or_name.into(),
            environment_id: None,
            generation_number: None,
            package_count: None,
        }
    }

    pub fn with_environment_id(mut self, environment_id: Uuid) -> Self {
        self.environment_id = Some(environment_id);
        self
    }

    pub fn with_generation_number(mut self, generation_number: u64) -> Self {
        self.generation_number = Some(generation_number);
        self
    }

    pub fn with_package_count(mut self, package_count: u64) -> Self {
        self.package_count = Some(package_count);
        self
    }
}

/// Payload shared by every environment event that carries env detail and
/// nothing else (push, pull, containerize, delete, include.upgrade,
/// install, list, uninstall, upgrade, the `services.*` events, and the
/// non-list `generations.*` events). The [`EventKind`] variant is the
/// discriminant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPayload {
    #[serde(flatten)]
    env_detail: EnvDetail,
}

impl CliEnvironmentPayload {
    pub fn new(env_detail: EnvDetail) -> Self {
        Self { env_detail }
    }
}

/// Payload for [`EventKind::CliEnvironmentActivate`].
///
/// Each `activate.rs` call site emits one event with only the extras it
/// knows populated; the downstream consumer correlates the rows via
/// `invocation_id` and coalesces the Optional fields (sparse merge).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentActivatePayload {
    #[serde(flatten)]
    env_detail: EnvDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_services: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_includes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lockfile_version: Option<String>,
    /// The locked manifest's declared schema version (`"1"`, `"1.10.0"`,
    /// …) — distinct from `lockfile_version`, the lockfile's own schema
    /// version. Absent on the eager emits; populated only on the
    /// result-known emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shell: Option<String>,
}

impl CliEnvironmentActivatePayload {
    /// Construct an empty-extras payload; call sites fill in the fields
    /// they know via the builder methods below.
    pub fn new(env_detail: EnvDetail) -> Self {
        Self {
            env_detail,
            start_services: None,
            mode: None,
            has_includes: None,
            lockfile_version: None,
            manifest_version: None,
            shell: None,
        }
    }

    pub fn with_start_services(mut self, value: bool) -> Self {
        self.start_services = Some(value);
        self
    }

    pub fn with_mode(mut self, value: impl Into<String>) -> Self {
        self.mode = Some(value.into());
        self
    }

    pub fn with_has_includes(mut self, value: bool) -> Self {
        self.has_includes = Some(value);
        self
    }

    pub fn with_lockfile_version(mut self, value: impl Into<String>) -> Self {
        self.lockfile_version = Some(value.into());
        self
    }

    pub fn with_manifest_version(mut self, value: impl Into<String>) -> Self {
        self.manifest_version = Some(value.into());
        self
    }

    pub fn with_shell(mut self, value: impl Into<String>) -> Self {
        self.shell = Some(value.into());
        self
    }
}

/// Outcome of an individual package's install / upgrade / uninstall
/// attempt within a single `flox <command>` invocation.
///
/// Recorded best-effort from a single error-handling site (e.g.
/// `Install::handle_error` in `cli/flox/src/commands/install.rs`); the
/// outcome value is ambiguous in two directions and consumers must
/// account for both:
///
/// - **Absence of `Success` is not proof of failure.** A `?`-propagated
///   early exit from inside the install pipeline skips the success-branch
///   emit; no per-package record is written for any package the
///   invocation attempted.
/// - **Presence of `Failure` is not proof that *that specific package*
///   failed.** On a partial-failure invocation the failure-branch emit
///   marks every attempted package `Failure` because the partition of
///   succeeded vs failed packages has not been computed at that site —
///   matching the legacy packed `failed_packages` string's semantics.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PackageOutcome {
    Success,
    Failure,
}

/// Payload shared by the per-package events (`cli.package.install` /
/// `.upgrade` / `.uninstall`). One event is emitted per package; see
/// [`PackageOutcome`] for the outcome semantics.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliPackagePayload {
    /// Per-package identifier matching what the legacy `failed_packages`
    /// string packed (catalog `pkg_path`, flake URL, or store path).
    package: String,
    outcome: PackageOutcome,
}

impl CliPackagePayload {
    pub fn new(package: String, outcome: PackageOutcome) -> Self {
        Self { package, outcome }
    }
}

// The payloads below carry per-command extras. Handlers with two emission
// sites (an eager env-detail emit + a result-known emit) follow the
// sparse-merge contract: both rows share `EventKind` and `invocation_id`,
// each populating only what it knows; the consumer coalesces.

/// Payload for [`EventKind::CliEnvironmentEdit`]. Emitted once eagerly
/// with env detail; a manifest edit that changes the manifest emits a
/// second row carrying `edited_includes`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentEditPayload {
    #[serde(flatten)]
    env_detail: EnvDetail,
    /// `true` when the edit produced a change to one of the included
    /// environments referenced by the manifest. `None` on the eager
    /// env-detail emit; `Some(bool)` on the result-known emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    edited_includes: Option<bool>,
    /// The edited manifest's declared schema version (`"1"`, `"1.10.0"`,
    /// …), from the post-edit lockfile. `None` on the eager emit — the
    /// manifest has not been loaded yet at that site.
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_version: Option<String>,
}

impl CliEnvironmentEditPayload {
    pub fn new(env_detail: EnvDetail) -> Self {
        Self {
            env_detail,
            edited_includes: None,
            manifest_version: None,
        }
    }

    pub fn with_edited_includes(mut self, value: bool) -> Self {
        self.edited_includes = Some(value);
        self
    }

    pub fn with_manifest_version(mut self, value: impl Into<String>) -> Self {
        self.manifest_version = Some(value.into());
        self
    }
}

/// Payload for [`EventKind::CliEnvironmentPublish`]. Emitted twice
/// per `flox publish` invocation per sparse-merge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPublishPayload {
    #[serde(flatten)]
    env_detail: EnvDetail,
    /// `true` when the manifest uses an `expression` build kind for
    /// the published package; `None` on the eager env-detail emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    has_expression_build: Option<bool>,
    /// `true` when the manifest uses a `manifest` build kind for the
    /// published package; `None` on the eager env-detail emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    has_manifest_build: Option<bool>,
    /// The published manifest's declared schema version (`"1"`,
    /// `"1.10.0"`, …), from the locked environment. `None` on the eager
    /// env-detail emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_version: Option<String>,
}

impl CliEnvironmentPublishPayload {
    pub fn new(env_detail: EnvDetail) -> Self {
        Self {
            env_detail,
            has_expression_build: None,
            has_manifest_build: None,
            manifest_version: None,
        }
    }

    pub fn with_build_kinds(
        mut self,
        has_expression_build: bool,
        has_manifest_build: bool,
    ) -> Self {
        self.has_expression_build = Some(has_expression_build);
        self.has_manifest_build = Some(has_manifest_build);
        self
    }

    pub fn with_manifest_version(mut self, value: impl Into<String>) -> Self {
        self.manifest_version = Some(value.into());
        self
    }
}

/// Payload for [`EventKind::CliEnvironmentGenerationsList`].
/// Carries env detail + `request_tree` (`true` when the user passed
/// `--tree`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentGenerationsListPayload {
    #[serde(flatten)]
    env_detail: EnvDetail,
    /// `true` when invoked with `--tree`.
    #[serde(skip_serializing_if = "Option::is_none")]
    request_tree: Option<bool>,
}

impl CliEnvironmentGenerationsListPayload {
    pub fn new(env_detail: EnvDetail) -> Self {
        Self {
            env_detail,
            request_tree: None,
        }
    }

    pub fn with_request_tree(mut self, value: bool) -> Self {
        self.request_tree = Some(value);
        self
    }
}

// `flox build` and `flox search` carry per-command extras but no
// environment context (build operates on the manifest's `build`
// table; search hits the catalog without a resolved environment).

/// Payload for [`EventKind::CliBuild`]. Carries `flox build`'s
/// per-invocation build-kind detection flags.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliBuildPayload {
    has_expression_build: bool,
    has_manifest_build: bool,
}

impl CliBuildPayload {
    pub fn new(has_expression_build: bool, has_manifest_build: bool) -> Self {
        Self {
            has_expression_build,
            has_manifest_build,
        }
    }
}

/// Payload for [`EventKind::CliSearch`]. Carries the user-supplied
/// search term verbatim, matching the legacy `subcommand_metric!(
/// "search", "search_term" = …)` extras.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliSearchPayload {
    search_term: String,
}

impl CliSearchPayload {
    pub fn new(search_term: String) -> Self {
        Self { search_term }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The wire form of `OffsetDateTime::from_unix_timestamp(0)` under
    /// `TimestampMilliSeconds<i64>` — milliseconds since the Unix
    /// epoch, where 1970-01-01T00:00:00Z is exactly 0.
    const EPOCH_UNIX_MS: i64 = 0;

    fn fixed_event(kind: EventKind) -> Event {
        Event {
            event_id: Uuid::nil(),
            event_timestamp: OffsetDateTime::from_unix_timestamp(0)
                .expect("0 is a valid unix timestamp"),
            source: "cli",
            invocation_id: Uuid::nil(),
            device_id: Uuid::nil(),
            auth_subject: None,
            kind,
        }
    }

    fn command_payload(subcommand: &str) -> CommandPayload {
        CommandPayload {
            subcommand: subcommand.to_string(),
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        }
    }

    fn expected_payload_json(subcommand: &str) -> serde_json::Value {
        json!({
            "subcommand": subcommand,
            "flox_version": "0.0.0-test",
            "os_family": "Linux",
            "os_family_release": "6.10.0",
            "os": "ubuntu",
            "os_version": "24.04",
            "empty_flags": [],
            "invocation_sources": ["shell"],
        })
    }

    fn command_run_envelope_json(payload: serde_json::Value) -> serde_json::Value {
        json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_run",
            "payload": payload,
        })
    }

    #[test]
    fn command_run_serializes_to_v2_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandRun(
            CliCommandRunPayload::new(command_payload("install")),
        )))
        .expect("event serializes");
        let expected = command_run_envelope_json(expected_payload_json("install"));
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_payload_without_lifecycle_fields_deserializes() {
        // A payload carrying only the subcommand (no lifecycle keys) must stay
        // field-additive and deserialize with `lifecycle: None`.
        let legacy = json!({ "subcommand": "install" });
        let payload: CliCommandCompletedPayload =
            serde_json::from_value(legacy).expect("payload without lifecycle deserializes");
        assert_eq!(payload, CliCommandCompletedPayload {
            subcommand: "install".to_string(),
            lifecycle: None,
        });
    }

    #[test]
    fn command_completed_payload_with_lifecycle_fields_deserializes() {
        // Buffered events are read back before delivery; the flattened
        // lifecycle must deserialize to `Some`, not silently collapse to
        // `None`.
        let json = json!({
            "subcommand": "install",
            "exit_code": 1,
            "duration_ms": 567,
            "error_kind": "environment.manifest_not_found",
        });
        let payload: CliCommandCompletedPayload =
            serde_json::from_value(json).expect("payload deserializes");
        assert_eq!(payload, CliCommandCompletedPayload {
            subcommand: "install".to_string(),
            lifecycle: Some(LifecycleFields {
                exit_code: 1,
                duration_ms: Some(567),
                error_kind: Some("environment.manifest_not_found".to_string()),
            }),
        });
    }

    #[test]
    fn command_completed_success_envelope_golden() {
        let payload = CliCommandCompletedPayload::new("install".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: Some(1234),
            error_kind: None,
        });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": {
                "subcommand": "install",
                "exit_code": 0,
                "duration_ms": 1234,
            },
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_handoff_records_exit_code_without_duration() {
        // The `activate` pre-exec handoff: exit_code 0, no completion duration.
        let payload = CliCommandCompletedPayload::new("activate".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: None,
            error_kind: None,
        });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let obj = value
            .get("payload")
            .and_then(|p| p.as_object())
            .expect("payload object");
        assert_eq!(obj.get("subcommand"), Some(&json!("activate")));
        assert_eq!(obj.get("exit_code"), Some(&json!(0)));
        assert!(
            !obj.contains_key("duration_ms"),
            "duration_ms should be omitted on handoff"
        );
    }

    #[test]
    fn command_completed_failure_envelope_golden() {
        let payload = CliCommandCompletedPayload::new("install".to_string(), LifecycleFields {
            exit_code: 1,
            duration_ms: Some(567),
            error_kind: Some("environment.manifest_not_found".to_string()),
        });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": {
                "subcommand": "install",
                "exit_code": 1,
                "duration_ms": 567,
                "error_kind": "environment.manifest_not_found",
            },
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn auth_subject_serializes_when_present() {
        let mut event = fixed_event(EventKind::CliCommandRun(CliCommandRunPayload::new(
            command_payload("install"),
        )));
        event.auth_subject = Some("test-subject-7f3a".to_string());
        let value = serde_json::to_value(event).expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "auth_subject": "test-subject-7f3a",
            "event_type": "cli.command_run",
            "payload": expected_payload_json("install"),
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn shared_metadata_template_merges_subcommand_into_payload() {
        let template = shared_metadata_for_payload_tests();
        let payload = template.into_payload("activate".to_string());
        assert_eq!(payload, command_payload("activate"));
    }

    /// Template fixture matching [`command_payload`].
    fn shared_metadata_for_payload_tests() -> SharedMetadataTemplate {
        SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        }
    }

    fn env_detail(kind: &str, ref_or_name: &str) -> EnvDetail {
        EnvDetail::new(kind, ref_or_name)
    }

    fn activate_envelope_json(payload: serde_json::Value) -> serde_json::Value {
        json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.activate",
            "payload": payload,
        })
    }

    #[test]
    fn cli_environment_activate_remote_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(env_detail("remote", "alice/myenv"))
            .with_start_services(false)
            .with_mode("dev");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "remote",
            "env_ref_or_name": "alice/myenv",
            "start_services": false,
            "mode": "dev",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_managed_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(env_detail("managed", "alice/myenv"))
            .with_has_includes(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
            "has_includes": true,
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_managed_identity_fields_envelope_golden() {
        let environment_id = Uuid::from_u128(0x11111111_1111_1111_1111_111111111111);
        let detail = env_detail("managed", "alice/myenv")
            .with_environment_id(environment_id)
            .with_generation_number(3)
            .with_package_count(7);
        let payload = CliEnvironmentActivatePayload::new(detail);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
            "environment_id": "11111111-1111-1111-1111-111111111111",
            "generation_number": 3,
            "package_count": 7,
        }));
        assert_eq!(value, expected);
    }

    /// Path environments have no generations: `generation_number` stays
    /// absent while the other identity fields ride.
    #[test]
    fn cli_environment_path_identity_fields_envelope_golden() {
        let environment_id = Uuid::from_u128(0x11111111_1111_1111_1111_111111111111);
        let detail = env_detail("path", "myenv")
            .with_environment_id(environment_id)
            .with_package_count(2);
        let payload = CliEnvironmentPayload::new(detail);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentList(payload)))
            .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.list",
            "payload": {
                "env_kind": "path",
                "env_ref_or_name": "myenv",
                "environment_id": "11111111-1111-1111-1111-111111111111",
                "package_count": 2,
            },
        });
        assert_eq!(value, expected);
    }

    /// Remote environments carry no `environment_id` (their cached
    /// pointer is rewritten per invocation) but do have generations and
    /// a lockfile.
    #[test]
    fn cli_environment_remote_identity_fields_envelope_golden() {
        let detail = env_detail("remote", "alice/myenv")
            .with_generation_number(5)
            .with_package_count(4);
        let payload = CliEnvironmentActivatePayload::new(detail);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "remote",
            "env_ref_or_name": "alice/myenv",
            "generation_number": 5,
            "package_count": 4,
        }));
        assert_eq!(value, expected);
    }

    /// A buffered environment-event row recorded by a binary that predates
    /// the identity fields must deserialize and re-serialize unchanged —
    /// absent fields stay absent, nothing is fabricated.
    #[test]
    fn environment_row_without_identity_fields_round_trips() {
        let old_row = env_envelope_json("cli.environment.delete");
        let event: Event =
            serde_json::from_value(old_row.clone()).expect("old-shape row deserializes");
        let EventKind::CliEnvironmentDelete(ref payload) = event.kind else {
            panic!("expected cli.environment.delete");
        };
        assert_eq!(payload, &CliEnvironmentPayload::new(managed_env_detail()));
        let reserialized = serde_json::to_value(event).expect("event re-serializes");
        assert_eq!(reserialized, old_row);
    }

    #[test]
    fn cli_environment_activate_path_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(env_detail("path", "myenv"))
            .with_lockfile_version("1")
            .with_manifest_version("1")
            .with_shell("bash");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "path",
            "env_ref_or_name": "myenv",
            "lockfile_version": "1",
            "manifest_version": "1",
            "shell": "bash",
        }));
        assert_eq!(value, expected);
    }

    /// The result-emit shape binaries predating `manifest_version`
    /// buffered (extras present, no `manifest_version`) must keep
    /// re-serializing without the new key.
    #[test]
    fn cli_environment_activate_without_manifest_version_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(env_detail("path", "myenv"))
            .with_lockfile_version("1")
            .with_shell("bash");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "path",
            "env_ref_or_name": "myenv",
            "lockfile_version": "1",
            "shell": "bash",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_activate_omits_unset_extras() {
        // No extras populated: every Optional field is `None` and the
        // wire shape omits them entirely (skip_serializing_if).
        let payload = CliEnvironmentActivatePayload::new(env_detail("path", "myenv"));
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentActivate(payload)))
            .expect("event serializes");
        let expected = activate_envelope_json(json!({
            "env_kind": "path",
            "env_ref_or_name": "myenv",
        }));
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_push_envelope_golden() {
        let payload = CliEnvironmentPayload::new(env_detail("managed", "alice/myenv"));
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPush(payload)))
            .expect("event serializes");
        let payload_json = json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
        });
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.push",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_pull_envelope_golden() {
        // The `NewAbbreviated` branch of `flox pull` constructs the detail
        // directly with `env_kind = "managed"`; assert that shape on the wire
        // so a future drift in the wrapper trips this test.
        let payload = CliEnvironmentPayload::new(env_detail("managed", "alice/myenv"));
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPull(payload)))
            .expect("event serializes");
        let payload_json = json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
        });
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.environment.pull",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }

    fn package_envelope_json(event_type: &str, package: &str, outcome: &str) -> serde_json::Value {
        let payload_json = json!({
            "package": package,
            "outcome": outcome,
        });
        json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": event_type,
            "payload": payload_json,
        })
    }

    #[test]
    fn cli_package_install_success_envelope_golden() {
        let payload = CliPackagePayload::new("hello".to_string(), PackageOutcome::Success);
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageInstall(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.install", "hello", "success");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_install_failure_envelope_golden() {
        let payload = CliPackagePayload::new("nope".to_string(), PackageOutcome::Failure);
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageInstall(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.install", "nope", "failure");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_upgrade_envelope_golden() {
        let payload = CliPackagePayload::new("hello".to_string(), PackageOutcome::Success);
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageUpgrade(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.upgrade", "hello", "success");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_uninstall_envelope_golden() {
        let payload = CliPackagePayload::new("hello".to_string(), PackageOutcome::Success);
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageUninstall(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.uninstall", "hello", "success");
        assert_eq!(value, expected);
    }

    /// Common helper for the env-detail-only envelope goldens: the expected
    /// envelope whose payload carries only the env-detail fields.
    fn env_envelope_json(event_type: &str) -> serde_json::Value {
        let payload_json = json!({
            "env_kind": "managed",
            "env_ref_or_name": "alice/myenv",
        });
        json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": event_type,
            "payload": payload_json,
        })
    }

    fn managed_env_detail() -> EnvDetail {
        env_detail("managed", "alice/myenv")
    }

    #[test]
    fn cli_environment_delete_envelope_golden() {
        let payload = CliEnvironmentPayload::new(managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentDelete(payload)))
            .expect("event serializes");
        let expected = env_envelope_json("cli.environment.delete");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_containerize_envelope_golden() {
        let payload = CliEnvironmentPayload::new(managed_env_detail());
        let value =
            serde_json::to_value(fixed_event(EventKind::CliEnvironmentContainerize(payload)))
                .expect("event serializes");
        let expected = env_envelope_json("cli.environment.containerize");
        assert_eq!(value, expected);
    }

    /// `cli.environment.include.upgrade` — the only `flox include`
    /// sub-command currently wiring a per-command event. The `.upgrade`
    /// suffix on `event_type` matches the `parent.child` shape used by
    /// every other nested-command event_type so a consumer
    /// `s/::/./g` between `subcommand` and `event_type` lines up
    /// element-for-element.
    #[test]
    fn cli_environment_include_upgrade_envelope_golden() {
        let payload = CliEnvironmentPayload::new(managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentIncludeUpgrade(
            payload,
        )))
        .expect("event serializes");
        let expected = env_envelope_json("cli.environment.include.upgrade");
        assert_eq!(value, expected);
    }

    /// Representative for the six `services::*` env-detail-only
    /// commands. The remaining five (`stop`/`restart`/`status`/`logs`/
    /// `persist`) share this exact shape — discriminating only on the
    /// `event_type` and `subcommand` fields.
    #[test]
    fn cli_environment_services_start_envelope_golden() {
        let payload = CliEnvironmentPayload::new(managed_env_detail());
        let value =
            serde_json::to_value(fixed_event(EventKind::CliEnvironmentServicesStart(payload)))
                .expect("event serializes");
        let expected = env_envelope_json("cli.environment.services.start");
        assert_eq!(value, expected);
    }

    /// Representative for the three env-detail-only `generations::*`
    /// commands (`history`/`rollback`/`switch`). The fourth member —
    /// `generations::list` — carries `request_tree` extras and has
    /// its own envelope test below.
    #[test]
    fn cli_environment_generations_history_envelope_golden() {
        let payload = CliEnvironmentPayload::new(managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentGenerationsHistory(
            payload,
        )))
        .expect("event serializes");
        let expected = env_envelope_json("cli.environment.generations.history");
        assert_eq!(value, expected);
    }

    /// `cli.environment.edit` on the eager call site: env-detail is
    /// populated, `edited_includes` defaults to `None` and is omitted
    /// from the wire shape via `skip_serializing_if`.
    #[test]
    fn cli_environment_edit_eager_envelope_golden() {
        let payload = CliEnvironmentEditPayload::new(managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentEdit(payload)))
            .expect("event serializes");
        let expected = env_envelope_json("cli.environment.edit");
        assert_eq!(value, expected);
    }

    /// `cli.environment.edit` on the result-known call site:
    /// `edited_includes` is populated and serializes as a top-level
    /// payload field. Sparse-merge with the eager event on the
    /// consumer side recovers the full row.
    #[test]
    fn cli_environment_edit_result_envelope_golden() {
        let payload = CliEnvironmentEditPayload::new(managed_env_detail())
            .with_edited_includes(true)
            .with_manifest_version("1");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentEdit(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.edit");
        let obj = expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object");
        obj.insert("edited_includes".to_string(), json!(true));
        obj.insert("manifest_version".to_string(), json!("1"));
        assert_eq!(value, expected);
    }

    /// The result-emit shape binaries predating `manifest_version`
    /// buffered (extras present, no `manifest_version`) must keep
    /// re-serializing without the new key.
    #[test]
    fn cli_environment_edit_result_without_manifest_version_envelope_golden() {
        let payload =
            CliEnvironmentEditPayload::new(managed_env_detail()).with_edited_includes(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentEdit(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.edit");
        expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object")
            .insert("edited_includes".to_string(), json!(true));
        assert_eq!(value, expected);
    }

    /// `cli.environment.publish` on the eager call site: every
    /// Optional extra is omitted from the wire.
    #[test]
    fn cli_environment_publish_eager_envelope_golden() {
        let payload = CliEnvironmentPublishPayload::new(managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPublish(payload)))
            .expect("event serializes");
        let expected = env_envelope_json("cli.environment.publish");
        assert_eq!(value, expected);
    }

    /// The result-emit shape binaries predating `manifest_version`
    /// buffered (build kinds present, no `manifest_version`) must keep
    /// re-serializing without the new key.
    #[test]
    fn cli_environment_publish_without_manifest_version_envelope_golden() {
        let payload =
            CliEnvironmentPublishPayload::new(managed_env_detail()).with_build_kinds(true, false);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPublish(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.publish");
        let obj = expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object");
        obj.insert("has_expression_build".to_string(), json!(true));
        obj.insert("has_manifest_build".to_string(), json!(false));
        assert_eq!(value, expected);
    }

    /// `cli.environment.publish` result-known emit carries both
    /// build-kind flags.
    #[test]
    fn cli_environment_publish_with_build_kinds_envelope_golden() {
        let payload = CliEnvironmentPublishPayload::new(managed_env_detail())
            .with_build_kinds(true, false)
            .with_manifest_version("1.10.0");
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPublish(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.publish");
        let obj = expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object");
        obj.insert("has_expression_build".to_string(), json!(true));
        obj.insert("has_manifest_build".to_string(), json!(false));
        obj.insert("manifest_version".to_string(), json!("1.10.0"));
        assert_eq!(value, expected);
    }

    /// `cli.environment.generations.list` carries `request_tree`
    /// reflecting the user-supplied `--tree` flag.
    #[test]
    fn cli_environment_generations_list_envelope_golden() {
        let payload =
            CliEnvironmentGenerationsListPayload::new(managed_env_detail()).with_request_tree(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentGenerationsList(
            payload,
        )))
        .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.generations.list");
        expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object")
            .insert("request_tree".to_string(), json!(true));
        assert_eq!(value, expected);
    }

    /// `cli.build` carries no env detail; both build-kind flags are
    /// non-Optional (required on the wire).
    #[test]
    fn cli_build_envelope_golden() {
        let payload = CliBuildPayload::new(true, false);
        let value = serde_json::to_value(fixed_event(EventKind::CliBuild(payload)))
            .expect("event serializes");
        let payload_json = json!({
            "has_expression_build": true,
            "has_manifest_build": false,
        });
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.build",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }

    /// `cli.search` carries the user-supplied search term verbatim
    /// and no env detail.
    #[test]
    fn cli_search_envelope_golden() {
        let payload = CliSearchPayload::new("ripgrep".to_string());
        let value = serde_json::to_value(fixed_event(EventKind::CliSearch(payload)))
            .expect("event serializes");
        let payload_json = json!({
            "search_term": "ripgrep",
        });
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.search",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }
}

#[cfg(test)]
mod pipeline_tests {
    use pretty_assertions::assert_eq;
    use serial_test::serial;
    use tempfile::TempDir;

    use super::*;
    use crate::test_helpers::MockEventsConnection;

    const DEVICE_ID: Uuid = Uuid::from_u128(0xaaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa);
    const INVOCATION_ID: Uuid = Uuid::from_u128(0xbbbbbbbb_bbbb_bbbb_bbbb_bbbbbbbbbbbb);

    fn fixed_event(kind: EventKind) -> Event {
        Event {
            event_id: Uuid::from_u128(0x11111111_1111_1111_1111_111111111111),
            event_timestamp: OffsetDateTime::from_unix_timestamp(1_700_000_000)
                .expect("fixture timestamp is valid"),
            source: "cli",
            invocation_id: INVOCATION_ID,
            device_id: DEVICE_ID,
            auth_subject: None,
            kind,
        }
    }

    fn shared_metadata() -> SharedMetadataTemplate {
        SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        }
    }

    fn command_run_kind() -> EventKind {
        EventKind::CliCommandRun(CliCommandRunPayload::new(
            shared_metadata().into_payload("install".to_string()),
        ))
    }

    fn command_completed_kind() -> EventKind {
        EventKind::CliCommandCompleted(CliCommandCompletedPayload::new(
            "install".to_string(),
            LifecycleFields {
                exit_code: 0,
                duration_ms: Some(1),
                error_kind: None,
            },
        ))
    }

    fn unix_timestamp_millis(time: OffsetDateTime) -> i128 {
        time.unix_timestamp_nanos() / 1_000_000
    }

    fn client_with_connection(tempdir: &TempDir, connection: MockEventsConnection) -> EventsClient {
        EventsClient::new_with_connection(
            DEVICE_ID,
            tempdir.path(),
            INVOCATION_ID,
            None,
            shared_metadata(),
            connection,
        )
    }

    /// A client constructed with an `auth_subject` stamps it on every
    /// recorded event; the anonymous helper above stamps `None`. Companion
    /// to the wire-shape goldens (`auth_subject_serializes_when_present`):
    /// this pins the *stamping* path from client state to envelope.
    #[test]
    fn client_stamps_auth_subject_on_recorded_events() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let client = EventsClient::new_with_connection(
            DEVICE_ID,
            tempdir.path(),
            INVOCATION_ID,
            Some("github|3670948".to_string()),
            shared_metadata(),
            MockEventsConnection::default(),
        );

        client
            .record_event(command_run_kind())
            .expect("record event");

        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        let events: Vec<_> = buffer.iter().collect();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].auth_subject.as_deref(),
            Some("github|3670948"),
            "client auth_subject must be stamped on the envelope"
        );
    }

    #[test]
    fn events_buffer_round_trips_entries_from_disk() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let first = fixed_event(command_run_kind());
        let second = fixed_event(command_completed_kind());

        let mut buffer = EventsBuffer::read(tempdir.path()).expect("read empty buffer");
        buffer.push(first.clone()).expect("push first event");
        buffer.push(second.clone()).expect("push second event");
        drop(buffer);

        let buffer = EventsBuffer::read(tempdir.path()).expect("read persisted buffer");

        assert_eq!(buffer.iter().cloned().collect::<Vec<_>>(), vec![
            first, second
        ]);
    }

    #[test]
    fn events_hub_without_client_skips_recording() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let hub = EventsHub::new();

        hub.record_event(command_run_kind())
            .expect("record with no client");

        assert!(!tempdir.path().join(EVENTS_BUFFER_FILE_NAME).exists());
    }

    #[test]
    fn events_hub_records_and_flushes_when_client_is_set() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_event(command_run_kind()).expect("record event");
        assert!(tempdir.path().join(EVENTS_BUFFER_FILE_NAME).exists());

        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 1);
        assert_eq!(sent_batches[0].len(), 1);
        assert_eq!(sent_batches[0][0].kind, command_run_kind());
    }

    #[test]
    fn events_client_record_stamps_event_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let client = client_with_connection(&tempdir, MockEventsConnection::default());
        let before = OffsetDateTime::now_utc();

        client
            .record_event(command_completed_kind())
            .expect("record event");

        let after = OffsetDateTime::now_utc();
        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        let event = buffer.iter().next().expect("one buffered event");

        assert_ne!(event.event_id, Uuid::nil());
        assert!(unix_timestamp_millis(event.event_timestamp) >= unix_timestamp_millis(before));
        assert!(unix_timestamp_millis(event.event_timestamp) <= unix_timestamp_millis(after));
        assert_eq!(event.source, "cli");
        assert_eq!(event.invocation_id, INVOCATION_ID);
        assert_eq!(event.device_id, DEVICE_ID);
        assert_eq!(event.auth_subject, None);
        assert_eq!(event.kind, command_completed_kind());
    }

    #[test]
    fn events_client_flush_batches_and_overwrites_buffer_file() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let mut client = client_with_connection(&tempdir, connection);

        for _ in 0..(BATCH_SIZE + 1) {
            client
                .record_event(command_run_kind())
                .expect("record event");
        }

        client.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.iter().map(Vec::len).collect::<Vec<_>>(), vec![
            BATCH_SIZE, 1
        ]);

        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        assert_eq!(buffer.iter().count(), 0);
        assert_eq!(
            std::fs::read_to_string(tempdir.path().join(EVENTS_BUFFER_FILE_NAME))
                .expect("read buffer file"),
            ""
        );
    }

    #[test]
    fn events_client_flush_retains_buffer_when_connection_errors() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        connection.fail_next_send();
        let mut client = client_with_connection(&tempdir, connection);

        client
            .record_event(command_run_kind())
            .expect("record event");

        let err = client.flush(true).expect_err("flush should fail");
        assert!(err.to_string().contains("mock events send failed"));

        let buffer = EventsBuffer::read(tempdir.path()).expect("read buffer");
        let buffered = buffer.iter().cloned().collect::<Vec<_>>();
        assert_eq!(buffered.len(), 1);
        assert_eq!(buffered[0].kind, command_run_kind());
    }

    /// Wire-contract test: the body for one event is exactly one JSON object
    /// (one NDJSON line), NOT a `[{...}]` array. An array body is the poison
    /// shape that stalls the S3Queue downstream — see
    /// `EventsConnectionV2::serialize_events` for the full rationale. Parallel
    /// fix to the same bug on the FloxHub side (floxhub@128dce329).
    #[test]
    fn v2_events_serializes_single_event_as_one_ndjson_object() {
        let event = fixed_event(command_run_kind());
        let body = EventsConnectionV2::serialize_events(&[&event]).expect("serialize events");

        // Wire contract: not an array.
        assert!(
            !body.starts_with('['),
            "body must not be a JSON array (would poison the S3Queue); got prefix: {:?}",
            &body[..body.len().min(48)]
        );
        // Exactly one JSON object on one line.
        assert!(body.starts_with('{') && body.ends_with('}'));
        assert!(
            !body.contains('\n'),
            "single-event body must be one line, no embedded \\n"
        );
        // This test pins the wire *shape*; the exact envelope bytes are
        // covered by the envelope serialization tests, and the payload grows
        // across PR 2b+, so it stays decoupled from payload contents.
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("single-event body parses as a JSON object");
        assert!(parsed.is_object());
        assert_eq!(parsed["event_type"], "cli.command_run");
    }

    /// A multi-event batch becomes one JSON object per line (`\n`-separated),
    /// so the API Gateway template's trailing-newline-appended Firehose Record
    /// lands as exactly N NDJSON lines in S3.
    #[test]
    fn v2_events_serializes_batch_as_ndjson_lines() {
        let e1 = fixed_event(command_run_kind());
        let e2 = fixed_event(command_run_kind());
        let body = EventsConnectionV2::serialize_events(&[&e1, &e2]).expect("serialize events");

        assert!(
            !body.starts_with('['),
            "batch body must not be a JSON array"
        );
        let lines: Vec<&str> = body.split('\n').collect();
        assert_eq!(lines.len(), 2, "two events must produce two NDJSON lines");
        for line in &lines {
            let parsed: serde_json::Value = serde_json::from_str(line).expect("parse line");
            assert!(parsed.is_object(), "each line must be a JSON object");
        }
    }

    /// Guard drop leaves unexpired events in the on-disk buffer for a later
    /// invocation to deliver, matching the legacy `MetricGuard`.
    #[test]
    #[serial(global_events_client)]
    fn events_guard_drop_defers_unexpired_events() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let previous_client =
            EventsHub::global().set_client(client_with_connection(&tempdir, connection));

        EventsHub::global()
            .record_event(command_run_kind())
            .expect("record event");
        drop(EventsGuard::new());

        EventsHub::global().clear_client();
        if let Some(previous_client) = previous_client {
            EventsHub::global().set_client(previous_client);
        }

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 0, "fresh events must not send on drop");
        let buffered = std::fs::read_to_string(tempdir.path().join(EVENTS_BUFFER_FILE_NAME))
            .expect("read buffer");
        assert_eq!(buffered.lines().count(), 1, "event stays buffered on disk");
    }

    /// A hub hands out at most one live guard, mirroring the legacy
    /// `Hub::try_guard` invariant. Uses a local hub so it neither touches nor
    /// races the global one.
    #[test]
    fn try_guard_rejects_a_second_active_guard() {
        let hub = EventsHub::new();
        let guard = hub.try_guard().expect("first guard is granted");
        // only one guard at a time
        assert!(
            hub.try_guard().is_err(),
            "a second guard must be refused while the first is live"
        );
        // dropping the first frees the slot
        drop(guard);
        assert!(
            hub.try_guard().is_ok(),
            "a guard is available again once the previous one is dropped"
        );
    }

    #[test]
    fn events_hub_record_command_run_stamps_subcommand_and_shared_metadata() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_command_run("activate".to_string())
            .expect("record command_run");
        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        assert_eq!(sent_batches.len(), 1);
        assert_eq!(sent_batches[0].len(), 1);
        match &sent_batches[0][0].kind {
            EventKind::CliCommandRun(payload) => {
                assert_eq!(
                    payload.command,
                    shared_metadata().into_payload("activate".to_string())
                );
            },
            other => panic!("expected CliCommandRun, got {other:?}"),
        }
    }

    #[test]
    fn events_hub_record_command_completed_is_idempotent_per_install() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_command_completed("install".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: Some(100),
            error_kind: None,
        })
        .expect("first completed record succeeds");
        hub.record_command_completed("install".to_string(), LifecycleFields {
            exit_code: 1,
            duration_ms: Some(200),
            error_kind: Some("environment.env_dir_not_found".to_string()),
        })
        .expect("second completed record is a silent no-op");
        hub.flush(true).expect("flush events");

        let sent_batches = sent_batches.lock().expect("sent batches lock").clone();
        let total_events: usize = sent_batches.iter().map(Vec::len).sum();
        assert_eq!(
            total_events, 1,
            "second record_command_completed must be a no-op"
        );
    }

    #[test]
    fn events_hub_set_client_resets_completed_recorded_flag() {
        let first_dir = tempfile::tempdir().expect("first tempdir");
        let second_dir = tempfile::tempdir().expect("second tempdir");
        let first_conn = MockEventsConnection::default();
        let second_conn = MockEventsConnection::default();
        let first_batches = first_conn.sent_batches();
        let second_batches = second_conn.sent_batches();

        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&first_dir, first_conn));
        hub.record_command_completed("install".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: Some(1),
            error_kind: None,
        })
        .unwrap();
        hub.flush(true).unwrap();
        hub.set_client(client_with_connection(&second_dir, second_conn));
        hub.record_command_completed("upgrade".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: Some(1),
            error_kind: None,
        })
        .expect("new install's completed record is allowed");
        hub.flush(true).unwrap();

        assert_eq!(
            first_batches
                .lock()
                .unwrap()
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            1
        );
        assert_eq!(
            second_batches
                .lock()
                .unwrap()
                .iter()
                .map(Vec::len)
                .sum::<usize>(),
            1
        );
    }

    /// Mirrors the spec's "install pkgA pkgB (all succeed)" partial-
    /// success case: one `cli.package.install` event per package with
    /// `outcome = success`, all sharing one `invocation_id`.
    #[test]
    fn package_install_all_succeed_one_event_per_package() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        hub.record_event(EventKind::CliPackageInstall(CliPackagePayload::new(
            "pkgA".to_string(),
            PackageOutcome::Success,
        )))
        .expect("record pkgA");
        hub.record_event(EventKind::CliPackageInstall(CliPackagePayload::new(
            "pkgB".to_string(),
            PackageOutcome::Success,
        )))
        .expect("record pkgB");
        hub.flush(true).expect("flush events");

        let events: Vec<_> = sent_batches
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(events.len(), 2);
        let mut packages = Vec::new();
        for event in &events {
            match &event.kind {
                EventKind::CliPackageInstall(p) => {
                    assert_eq!(p.outcome, PackageOutcome::Success);
                    packages.push(p.package.clone());
                },
                other => panic!("expected CliPackageInstall, got {other:?}"),
            }
            assert_eq!(event.invocation_id, INVOCATION_ID);
        }
        packages.sort();
        assert_eq!(packages, vec!["pkgA".to_string(), "pkgB".to_string()]);
    }

    /// Mirrors the spec's "install pkgA nope" partial-failure case: the
    /// new pipeline's failure-path emit records every attempted package
    /// (the `?`-propagated equivalent the spec calls out), not just the
    /// one that failed.
    #[test]
    fn package_install_failure_path_records_every_attempted_package() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        // Simulating `Install::handle_error` recording an event for every
        // attempted package on a mid-pipeline failure.
        for package in ["pkgA", "nope"] {
            hub.record_event(EventKind::CliPackageInstall(CliPackagePayload::new(
                package.to_string(),
                PackageOutcome::Failure,
            )))
            .expect("record failure");
        }
        hub.flush(true).expect("flush events");

        let events: Vec<_> = sent_batches
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(events.len(), 2);
        for event in &events {
            match &event.kind {
                EventKind::CliPackageInstall(p) => {
                    assert_eq!(p.outcome, PackageOutcome::Failure);
                },
                other => panic!("expected CliPackageInstall, got {other:?}"),
            }
        }
    }

    /// `cli.environment.edit` follows the sparse-merge contract:
    /// the eager call site emits with env-detail only; the result-
    /// known site emits with `edited_includes` populated. Both events
    /// share `invocation_id` and `event_type` so the consumer's
    /// `GROUP BY invocation_id, event_type` + `COALESCE` recovers a
    /// single `cli.telemetry` row.
    #[test]
    fn environment_edit_sparse_merge_emits_two_events_per_invocation() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let connection = MockEventsConnection::default();
        let sent_batches = connection.sent_batches();
        let hub = EventsHub::new();
        hub.set_client(client_with_connection(&tempdir, connection));

        let env_detail = EnvDetail::new("path", "myenv");

        hub.record_event(EventKind::CliEnvironmentEdit(
            CliEnvironmentEditPayload::new(env_detail.clone()),
        ))
        .expect("eager emit");
        hub.record_event(EventKind::CliEnvironmentEdit(
            CliEnvironmentEditPayload::new(env_detail).with_edited_includes(true),
        ))
        .expect("result-known emit");
        hub.flush(true).expect("flush events");

        let events: Vec<_> = sent_batches
            .lock()
            .unwrap()
            .clone()
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(events.len(), 2);
        let mut eager_seen = false;
        let mut result_seen = false;
        for event in &events {
            assert_eq!(event.invocation_id, INVOCATION_ID);
            match &event.kind {
                EventKind::CliEnvironmentEdit(p) => {
                    assert_eq!(p.env_detail.env_kind, "path");
                    match p.edited_includes {
                        None => eager_seen = true,
                        Some(true) => result_seen = true,
                        Some(false) => panic!("unexpected edited_includes=false"),
                    }
                },
                other => panic!("expected CliEnvironmentEdit, got {other:?}"),
            }
        }
        assert!(eager_seen, "missing eager env-detail-only event");
        assert!(
            result_seen,
            "missing result-known event with edited_includes"
        );
    }
}
