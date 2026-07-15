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
/// truth for each variant; call sites use the enum, never a string
/// literal. `derive_more::From` is derived so a call site can pass a
/// payload value directly to anything accepting `impl Into<EventKind>`.
#[derive(Debug, Clone, Serialize, Deserialize, derive_more::From, PartialEq, Eq)]
#[serde(tag = "event_type", content = "payload")]
pub enum EventKind {
    #[serde(rename = "cli.command_run")]
    CliCommandRun(CliCommandRunPayload),
    #[serde(rename = "cli.command_completed")]
    CliCommandCompleted(CliCommandCompletedPayload),
    #[serde(rename = "cli.environment.activate")]
    CliEnvironmentActivate(CliEnvironmentActivatePayload),
    #[serde(rename = "cli.environment.push")]
    CliEnvironmentPush(CliEnvironmentPushPayload),
    #[serde(rename = "cli.environment.pull")]
    CliEnvironmentPull(CliEnvironmentPullPayload),
    #[serde(rename = "cli.package.install")]
    CliPackageInstall(CliPackageInstallPayload),
    #[serde(rename = "cli.package.upgrade")]
    CliPackageUpgrade(CliPackageUpgradePayload),
    #[serde(rename = "cli.package.uninstall")]
    CliPackageUninstall(CliPackageUninstallPayload),
    #[serde(rename = "cli.environment.containerize")]
    CliEnvironmentContainerize(CliEnvironmentContainerizePayload),
    #[serde(rename = "cli.environment.delete")]
    CliEnvironmentDelete(CliEnvironmentDeletePayload),
    #[serde(rename = "cli.environment.edit")]
    CliEnvironmentEdit(CliEnvironmentEditPayload),
    #[serde(rename = "cli.environment.include.upgrade")]
    CliEnvironmentIncludeUpgrade(CliEnvironmentIncludeUpgradePayload),
    #[serde(rename = "cli.environment.install")]
    CliEnvironmentInstall(CliEnvironmentInstallPayload),
    #[serde(rename = "cli.environment.list")]
    CliEnvironmentList(CliEnvironmentListPayload),
    #[serde(rename = "cli.environment.publish")]
    CliEnvironmentPublish(CliEnvironmentPublishPayload),
    #[serde(rename = "cli.environment.uninstall")]
    CliEnvironmentUninstall(CliEnvironmentUninstallPayload),
    #[serde(rename = "cli.environment.upgrade")]
    CliEnvironmentUpgrade(CliEnvironmentUpgradePayload),
    #[serde(rename = "cli.environment.services.start")]
    CliEnvironmentServicesStart(CliEnvironmentServicesStartPayload),
    #[serde(rename = "cli.environment.services.stop")]
    CliEnvironmentServicesStop(CliEnvironmentServicesStopPayload),
    #[serde(rename = "cli.environment.services.restart")]
    CliEnvironmentServicesRestart(CliEnvironmentServicesRestartPayload),
    #[serde(rename = "cli.environment.services.status")]
    CliEnvironmentServicesStatus(CliEnvironmentServicesStatusPayload),
    #[serde(rename = "cli.environment.services.logs")]
    CliEnvironmentServicesLogs(CliEnvironmentServicesLogsPayload),
    #[serde(rename = "cli.environment.services.persist")]
    CliEnvironmentServicesPersist(CliEnvironmentServicesPersistPayload),
    #[serde(rename = "cli.environment.generations.history")]
    CliEnvironmentGenerationsHistory(CliEnvironmentGenerationsHistoryPayload),
    #[serde(rename = "cli.environment.generations.list")]
    CliEnvironmentGenerationsList(CliEnvironmentGenerationsListPayload),
    #[serde(rename = "cli.environment.generations.rollback")]
    CliEnvironmentGenerationsRollback(CliEnvironmentGenerationsRollbackPayload),
    #[serde(rename = "cli.environment.generations.switch")]
    CliEnvironmentGenerationsSwitch(CliEnvironmentGenerationsSwitchPayload),
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
    pub subcommand: String,
    /// Flox CLI version string.
    pub flox_version: String,
    /// Coarse operating system family (e.g. `Mac OS`, `Linux`).
    pub os_family: Option<String>,
    /// OS family release version.
    pub os_family_release: Option<String>,
    /// Linux distribution id (e.g. `ubuntu`); `None` outside Linux.
    pub os: Option<String>,
    /// Linux distribution version (e.g. `22.04`); `None` outside Linux.
    pub os_version: Option<String>,
    /// CLI flags that were observed empty on this invocation. Reserved for
    /// the per-command instrumentation PRs.
    pub empty_flags: Vec<String>,
    /// Tokens describing how this CLI invocation was launched (shell, prompt,
    /// service runner, etc.). Mirrors the legacy `INVOCATION_SOURCES`.
    pub invocation_sources: Vec<String>,
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

/// Payload for [`EventKind::CliCommandRun`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandRunPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
}

impl CliCommandRunPayload {
    pub fn new(command: CommandPayload) -> Self {
        Self { command }
    }
}

/// PII-safe descriptor of a failed dispatch. Callers must derive both values
/// from a fixed set of compile-time strings (never from a rendered error) so
/// user data cannot reach telemetry.
///
/// Serializes flattened into the payload as `error_kind` / `error_message`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LifecycleError {
    /// Bounded operation slug (e.g. `env_not_found`).
    #[serde(rename = "error_kind")]
    pub kind: String,
    /// Short static descriptor for the same failure.
    #[serde(rename = "error_message")]
    pub message: String,
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
    /// PII-safe descriptor of the failure; `None` on success.
    #[serde(flatten)]
    pub error: Option<LifecycleError>,
}

/// Payload for [`EventKind::CliCommandCompleted`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliCommandCompletedPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    /// `None` only on events recorded by clients that predate lifecycle
    /// reporting, which carry none of the lifecycle keys; the flatten keeps
    /// the wire shape field-additive for them.
    #[serde(flatten)]
    pub lifecycle: Option<LifecycleFields>,
}

impl CliCommandCompletedPayload {
    pub fn new(command: CommandPayload, lifecycle: LifecycleFields) -> Self {
        Self {
            command,
            lifecycle: Some(lifecycle),
        }
    }
}

/// Environment kind a `flox activate` / `push` / `pull` invocation touched,
/// matching the three legacy `environment_subcommand_metric!` arms
/// (`remote_environment` / `managed_environment` / `path_environment`).
///
/// Carried on every `cli.environment.*` event so downstream classifiers can
/// reconstruct the legacy `*_environment` columns without re-parsing the
/// `event_type` tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvDetail {
    /// One of `"remote"`, `"managed"`, `"path"` — the
    /// [`flox_rust_sdk::models::environment::ConcreteEnvironment`] variant
    /// the command operated on. `"managed"` is also used for `flox pull`'s
    /// `NewAbbreviated` branch, where only the remote `RemoteRef` is known
    /// at emission time (no materialized `ConcreteEnvironment` yet).
    pub env_kind: String,
    /// The environment's identifier — the result of `env_ref().to_string()`
    /// for remote and managed environments, and `Environment::name(...)`
    /// for path environments. Matches the value the legacy macros emit.
    pub env_ref_or_name: String,
}

/// Payload for [`EventKind::CliEnvironmentActivate`].
///
/// Carries the env detail plus the extras the legacy
/// `environment_subcommand_metric!("activate", ...)` and
/// `subcommand_metric!("activate", ...)` call sites in
/// `cli/flox/src/commands/activate.rs` recorded. Each call site emits one
/// event with only the fields it knows populated; the downstream consumer
/// correlates via `invocation_id`. `lockfile_version` lands here instead of
/// on a (dropped) `cli.environment.activate#version` pseudo-subcommand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentActivatePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_services: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_includes: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lockfile_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

impl CliEnvironmentActivatePayload {
    /// Construct an empty-extras payload — every Optional field defaulted
    /// to `None`. Call sites fill in the fields they know via the builder
    /// methods below or struct-literal field overrides.
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
            start_services: None,
            mode: None,
            has_includes: None,
            lockfile_version: None,
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

    pub fn with_shell(mut self, value: impl Into<String>) -> Self {
        self.shell = Some(value.into());
        self
    }
}

/// Payload for [`EventKind::CliEnvironmentPush`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPushPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentPushPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentPull`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPullPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentPullPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
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

/// Payload for [`EventKind::CliPackageInstall`]. One event is emitted per
/// package on the success branch (with `PackageOutcome::Success`) and per
/// package on the failure branch (with `PackageOutcome::Failure`) — see
/// the `cli/flox/src/commands/install.rs` call sites for the emission
/// points.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliPackageInstallPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    /// Per-package identifier matching what
    /// `Install::format_packages_for_tracing` joins into the legacy
    /// `failed_packages` string (catalog `pkg_path`, flake URL, or store
    /// path).
    pub package: String,
    pub outcome: PackageOutcome,
}

impl CliPackageInstallPayload {
    pub fn new(command: CommandPayload, package: String, outcome: PackageOutcome) -> Self {
        Self {
            command,
            package,
            outcome,
        }
    }
}

/// Payload for [`EventKind::CliPackageUpgrade`]. One event per
/// upgraded package on the success branch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliPackageUpgradePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    pub package: String,
    pub outcome: PackageOutcome,
}

impl CliPackageUpgradePayload {
    pub fn new(command: CommandPayload, package: String, outcome: PackageOutcome) -> Self {
        Self {
            command,
            package,
            outcome,
        }
    }
}

/// Payload for [`EventKind::CliPackageUninstall`]. One event per
/// removed package on the success branch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliPackageUninstallPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    pub package: String,
    pub outcome: PackageOutcome,
}

impl CliPackageUninstallPayload {
    pub fn new(command: CommandPayload, package: String, outcome: PackageOutcome) -> Self {
        Self {
            command,
            package,
            outcome,
        }
    }
}

// The env-detail-only payloads below carry `CommandPayload` +
// `EnvDetail` and nothing more. The structs are byte-identical to
// `CliEnvironmentPushPayload`; they exist as separate types so each
// `EventKind` variant owns a distinct payload type. A future cleanup
// may collapse them into a shared `EnvCommandPayload`.

/// Payload for [`EventKind::CliEnvironmentContainerize`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentContainerizePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentContainerizePayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentDelete`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentDeletePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentDeletePayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentIncludeUpgrade`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentIncludeUpgradePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentIncludeUpgradePayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentInstall`]. Carries the
/// env-detail row of a `flox install` invocation; the per-package
/// detail rides on [`EventKind::CliPackageInstall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentInstallPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentInstallPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentList`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentListPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentListPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentUninstall`]. Carries the
/// env-detail row of a `flox uninstall` invocation; the per-package
/// detail rides on [`EventKind::CliPackageUninstall`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentUninstallPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentUninstallPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentUpgrade`]. Carries the
/// env-detail row of a `flox upgrade` invocation; the per-package
/// detail rides on [`EventKind::CliPackageUpgrade`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentUpgradePayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentUpgradePayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesStart`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesStartPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesStartPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesStop`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesStopPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesStopPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesRestart`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesRestartPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesRestartPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesStatus`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesStatusPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesStatusPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesLogs`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesLogsPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesLogsPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentServicesPersist`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentServicesPersistPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentServicesPersistPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentGenerationsHistory`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentGenerationsHistoryPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentGenerationsHistoryPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentGenerationsRollback`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentGenerationsRollbackPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentGenerationsRollbackPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

/// Payload for [`EventKind::CliEnvironmentGenerationsSwitch`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentGenerationsSwitchPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
}

impl CliEnvironmentGenerationsSwitchPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
        }
    }
}

// The payloads below carry both env-detail and per-command extras,
// and have two distinct legacy emission sites in the same handler
// (an eager env-detail emit before the operation runs + an extras
// emit after the operation result is known). The new path follows a
// sparse-merge contract: both sites emit a payload with the same
// `EventKind` and same `invocation_id`, each populating only what it
// knows. The consumer `COALESCE`s Optional fields across the rows.

/// Payload for [`EventKind::CliEnvironmentEdit`]. Emitted once eagerly
/// with env detail; a manifest edit that changes the manifest emits a
/// second row carrying `edited_includes`. The other edit actions
/// (rename/sync/reset, or an unchanged manifest edit) emit only the
/// eager row — per the sparse-merge contract above, `edited_includes`
/// is simply absent for those.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentEditPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
    /// `true` when the edit produced a change to one of the included
    /// environments referenced by the manifest. `None` on the eager
    /// env-detail emit (before the edit runs); `Some(bool)` on the
    /// result-known emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited_includes: Option<bool>,
}

impl CliEnvironmentEditPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
            edited_includes: None,
        }
    }

    pub fn with_edited_includes(mut self, value: bool) -> Self {
        self.edited_includes = Some(value);
        self
    }
}

/// Payload for [`EventKind::CliEnvironmentPublish`]. Emitted twice
/// per `flox publish` invocation per sparse-merge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentPublishPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
    /// `true` when the manifest uses an `expression` build kind for
    /// the published package; `None` on the eager env-detail emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_expression_build: Option<bool>,
    /// `true` when the manifest uses a `manifest` build kind for the
    /// published package; `None` on the eager env-detail emit.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub has_manifest_build: Option<bool>,
}

impl CliEnvironmentPublishPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
            env_detail,
            has_expression_build: None,
            has_manifest_build: None,
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
}

/// Payload for [`EventKind::CliEnvironmentGenerationsList`].
/// Carries env detail + `request_tree` (`true` when the user passed
/// `--tree`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CliEnvironmentGenerationsListPayload {
    #[serde(flatten)]
    pub command: CommandPayload,
    #[serde(flatten)]
    pub env_detail: EnvDetail,
    /// `true` when invoked with `--tree`; `None` is unused (single
    /// call site populates this on the eager env-detail emit).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_tree: Option<bool>,
}

impl CliEnvironmentGenerationsListPayload {
    pub fn new(command: CommandPayload, env_detail: EnvDetail) -> Self {
        Self {
            command,
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
    #[serde(flatten)]
    pub command: CommandPayload,
    pub has_expression_build: bool,
    pub has_manifest_build: bool,
}

impl CliBuildPayload {
    pub fn new(
        command: CommandPayload,
        has_expression_build: bool,
        has_manifest_build: bool,
    ) -> Self {
        Self {
            command,
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
    #[serde(flatten)]
    pub command: CommandPayload,
    pub search_term: String,
}

impl CliSearchPayload {
    pub fn new(command: CommandPayload, search_term: String) -> Self {
        Self {
            command,
            search_term,
        }
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

    #[test]
    fn command_run_serializes_to_v2_envelope() {
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandRun(
            CliCommandRunPayload::new(command_payload("install")),
        )))
        .expect("event serializes");
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_run",
            "payload": expected_payload_json("install"),
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_payload_without_lifecycle_fields_deserializes() {
        // Events buffered by clients that predate lifecycle reporting carry
        // none of the lifecycle fields; the payload must stay field-additive.
        let legacy = expected_payload_json("install");
        let payload: CliCommandCompletedPayload =
            serde_json::from_value(legacy).expect("legacy payload deserializes");
        assert_eq!(payload, CliCommandCompletedPayload {
            command: command_payload("install"),
            lifecycle: None,
        });
    }

    #[test]
    fn command_completed_payload_with_lifecycle_fields_deserializes() {
        // Buffered events are read back before delivery; the flattened
        // lifecycle (including the flattened error pair) must deserialize to
        // `Some`, not silently collapse to `None`.
        let mut json = expected_payload_json("install");
        let obj = json.as_object_mut().expect("payload object");
        obj.insert("exit_code".to_string(), json!(1));
        obj.insert("duration_ms".to_string(), json!(567));
        obj.insert("error_kind".to_string(), json!("catalog_resolve"));
        obj.insert(
            "error_message".to_string(),
            json!("failed to resolve packages from catalog"),
        );
        let payload: CliCommandCompletedPayload =
            serde_json::from_value(json).expect("payload deserializes");
        assert_eq!(payload, CliCommandCompletedPayload {
            command: command_payload("install"),
            lifecycle: Some(LifecycleFields {
                exit_code: 1,
                duration_ms: Some(567),
                error: Some(LifecycleError {
                    kind: "catalog_resolve".to_string(),
                    message: "failed to resolve packages from catalog".to_string(),
                }),
            }),
        });
    }

    #[test]
    fn command_completed_success_envelope_golden() {
        let payload =
            CliCommandCompletedPayload::new(command_payload("install"), LifecycleFields {
                exit_code: 0,
                duration_ms: Some(1234),
                error: None,
            });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("install");
        let obj = payload_json.as_object_mut().expect("payload object");
        obj.insert("exit_code".to_string(), json!(0));
        obj.insert("duration_ms".to_string(), json!(1234));
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": payload_json,
        });
        assert_eq!(value, expected);
    }

    #[test]
    fn command_completed_handoff_records_exit_code_without_duration() {
        // The `activate` pre-exec handoff: exit_code 0, no completion duration.
        let payload =
            CliCommandCompletedPayload::new(command_payload("activate"), LifecycleFields {
                exit_code: 0,
                duration_ms: None,
                error: None,
            });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let obj = value
            .get("payload")
            .and_then(|p| p.as_object())
            .expect("payload object");
        assert_eq!(obj.get("exit_code"), Some(&json!(0)));
        assert!(
            !obj.contains_key("duration_ms"),
            "duration_ms should be omitted on handoff"
        );
    }

    #[test]
    fn command_completed_failure_envelope_golden() {
        let payload =
            CliCommandCompletedPayload::new(command_payload("install"), LifecycleFields {
                exit_code: 1,
                duration_ms: Some(567),
                error: Some(LifecycleError {
                    kind: "catalog_resolve".to_string(),
                    message: "failed to resolve packages from catalog".to_string(),
                }),
            });
        let value = serde_json::to_value(fixed_event(EventKind::CliCommandCompleted(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("install");
        let obj = payload_json.as_object_mut().expect("payload object");
        obj.insert("exit_code".to_string(), json!(1));
        obj.insert("duration_ms".to_string(), json!(567));
        obj.insert("error_kind".to_string(), json!("catalog_resolve"));
        obj.insert(
            "error_message".to_string(),
            json!("failed to resolve packages from catalog"),
        );
        let expected = json!({
            "event_id": "00000000-0000-0000-0000-000000000000",
            "event_timestamp": EPOCH_UNIX_MS,
            "source": "cli",
            "invocation_id": "00000000-0000-0000-0000-000000000000",
            "device_id": "00000000-0000-0000-0000-000000000000",
            "event_type": "cli.command_completed",
            "payload": payload_json,
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
        let template = SharedMetadataTemplate {
            flox_version: "0.0.0-test".to_string(),
            os_family: Some("Linux".to_string()),
            os_family_release: Some("6.10.0".to_string()),
            os: Some("ubuntu".to_string()),
            os_version: Some("24.04".to_string()),
            empty_flags: vec![],
            invocation_sources: vec!["shell".to_string()],
        };
        let payload = template.into_payload("activate".to_string());
        assert_eq!(payload, command_payload("activate"));
    }

    fn env_detail(kind: &str, ref_or_name: &str) -> EnvDetail {
        EnvDetail {
            env_kind: kind.to_string(),
            env_ref_or_name: ref_or_name.to_string(),
        }
    }

    fn activate_envelope_json(payload_extras: serde_json::Value) -> serde_json::Value {
        let mut payload = expected_payload_json("activate");
        let payload_obj = payload.as_object_mut().expect("payload object");
        for (key, value) in payload_extras.as_object().expect("extras object") {
            payload_obj.insert(key.clone(), value.clone());
        }
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
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("remote", "alice/myenv"),
        )
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
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("managed", "alice/myenv"),
        )
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
    fn cli_environment_activate_path_envelope_golden() {
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("path", "myenv"),
        )
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
        let payload = CliEnvironmentActivatePayload::new(
            command_payload("activate"),
            env_detail("path", "myenv"),
        );
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
        let payload = CliEnvironmentPushPayload::new(
            command_payload("push"),
            env_detail("managed", "alice/myenv"),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPush(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("push");
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("env_kind".to_string(), json!("managed"));
        payload_obj.insert("env_ref_or_name".to_string(), json!("alice/myenv"));
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
        // The `NewAbbreviated` branch in `pull.rs:103` constructs the
        // detail directly with `env_kind = "managed"`; assert that
        // shape on the wire so a future drift in the wrapper trips
        // this test.
        let payload = CliEnvironmentPullPayload::new(
            command_payload("pull"),
            env_detail("managed", "alice/myenv"),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPull(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("pull");
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("env_kind".to_string(), json!("managed"));
        payload_obj.insert("env_ref_or_name".to_string(), json!("alice/myenv"));
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

    fn package_envelope_json(
        event_type: &str,
        subcommand: &str,
        package: &str,
        outcome: &str,
    ) -> serde_json::Value {
        let mut payload_json = expected_payload_json(subcommand);
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("package".to_string(), json!(package));
        payload_obj.insert("outcome".to_string(), json!(outcome));
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
        let payload = CliPackageInstallPayload::new(
            command_payload("install"),
            "hello".to_string(),
            PackageOutcome::Success,
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageInstall(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.install", "install", "hello", "success");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_install_failure_envelope_golden() {
        let payload = CliPackageInstallPayload::new(
            command_payload("install"),
            "nope".to_string(),
            PackageOutcome::Failure,
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageInstall(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.install", "install", "nope", "failure");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_upgrade_envelope_golden() {
        let payload = CliPackageUpgradePayload::new(
            command_payload("upgrade"),
            "hello".to_string(),
            PackageOutcome::Success,
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageUpgrade(payload)))
            .expect("event serializes");
        let expected = package_envelope_json("cli.package.upgrade", "upgrade", "hello", "success");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_package_uninstall_envelope_golden() {
        let payload = CliPackageUninstallPayload::new(
            command_payload("uninstall"),
            "hello".to_string(),
            PackageOutcome::Success,
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliPackageUninstall(payload)))
            .expect("event serializes");
        let expected =
            package_envelope_json("cli.package.uninstall", "uninstall", "hello", "success");
        assert_eq!(value, expected);
    }

    /// Common helper for the env-detail-only envelope goldens.
    /// Builds an `Event` from `subcommand` + v2 env-detail
    /// fields and the expected JSON shape it should serialize to.
    fn env_envelope_json(event_type: &str, subcommand: &str) -> serde_json::Value {
        let mut payload_json = expected_payload_json(subcommand);
        let payload_obj = payload_json.as_object_mut().expect("payload object");
        payload_obj.insert("env_kind".to_string(), json!("managed"));
        payload_obj.insert("env_ref_or_name".to_string(), json!("alice/myenv"));
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
        let payload =
            CliEnvironmentDeletePayload::new(command_payload("delete"), managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentDelete(payload)))
            .expect("event serializes");
        let expected = env_envelope_json("cli.environment.delete", "delete");
        assert_eq!(value, expected);
    }

    #[test]
    fn cli_environment_containerize_envelope_golden() {
        let payload = CliEnvironmentContainerizePayload::new(
            command_payload("containerize"),
            managed_env_detail(),
        );
        let value =
            serde_json::to_value(fixed_event(EventKind::CliEnvironmentContainerize(payload)))
                .expect("event serializes");
        let expected = env_envelope_json("cli.environment.containerize", "containerize");
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
        let payload = CliEnvironmentIncludeUpgradePayload::new(
            command_payload("include::upgrade"),
            managed_env_detail(),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentIncludeUpgrade(
            payload,
        )))
        .expect("event serializes");
        let expected = env_envelope_json("cli.environment.include.upgrade", "include::upgrade");
        assert_eq!(value, expected);
    }

    /// Representative for the six `services::*` env-detail-only
    /// commands. The remaining five (`stop`/`restart`/`status`/`logs`/
    /// `persist`) share this exact shape — discriminating only on the
    /// `event_type` and `subcommand` fields.
    #[test]
    fn cli_environment_services_start_envelope_golden() {
        let payload = CliEnvironmentServicesStartPayload::new(
            command_payload("services::start"),
            managed_env_detail(),
        );
        let value =
            serde_json::to_value(fixed_event(EventKind::CliEnvironmentServicesStart(payload)))
                .expect("event serializes");
        let expected = env_envelope_json("cli.environment.services.start", "services::start");
        assert_eq!(value, expected);
    }

    /// Representative for the three env-detail-only `generations::*`
    /// commands (`history`/`rollback`/`switch`). The fourth member —
    /// `generations::list` — carries `request_tree` extras and has
    /// its own envelope test below.
    #[test]
    fn cli_environment_generations_history_envelope_golden() {
        let payload = CliEnvironmentGenerationsHistoryPayload::new(
            command_payload("generations::history"),
            managed_env_detail(),
        );
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentGenerationsHistory(
            payload,
        )))
        .expect("event serializes");
        let expected = env_envelope_json(
            "cli.environment.generations.history",
            "generations::history",
        );
        assert_eq!(value, expected);
    }

    /// `cli.environment.edit` on the eager call site: env-detail is
    /// populated, `edited_includes` defaults to `None` and is omitted
    /// from the wire shape via `skip_serializing_if`.
    #[test]
    fn cli_environment_edit_eager_envelope_golden() {
        let payload = CliEnvironmentEditPayload::new(command_payload("edit"), managed_env_detail());
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentEdit(payload)))
            .expect("event serializes");
        let expected = env_envelope_json("cli.environment.edit", "edit");
        assert_eq!(value, expected);
    }

    /// `cli.environment.edit` on the result-known call site:
    /// `edited_includes` is populated and serializes as a top-level
    /// payload field. Sparse-merge with the eager event on the
    /// consumer side recovers the full row.
    #[test]
    fn cli_environment_edit_result_envelope_golden() {
        let payload = CliEnvironmentEditPayload::new(command_payload("edit"), managed_env_detail())
            .with_edited_includes(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentEdit(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.edit", "edit");
        expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object")
            .insert("edited_includes".to_string(), json!(true));
        assert_eq!(value, expected);
    }

    /// `cli.environment.publish` result-known emit carries both
    /// build-kind flags. The eager emit shape (extras omitted) is
    /// covered by [`cli_environment_edit_eager_envelope_golden`]'s
    /// pattern — they share the same `skip_serializing_if` behavior.
    #[test]
    fn cli_environment_publish_with_build_kinds_envelope_golden() {
        let payload =
            CliEnvironmentPublishPayload::new(command_payload("publish"), managed_env_detail())
                .with_build_kinds(true, false);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentPublish(payload)))
            .expect("event serializes");
        let mut expected = env_envelope_json("cli.environment.publish", "publish");
        let obj = expected
            .get_mut("payload")
            .and_then(|p| p.as_object_mut())
            .expect("payload object");
        obj.insert("has_expression_build".to_string(), json!(true));
        obj.insert("has_manifest_build".to_string(), json!(false));
        assert_eq!(value, expected);
    }

    /// `cli.environment.generations.list` carries `request_tree`
    /// reflecting the user-supplied `--tree` flag.
    #[test]
    fn cli_environment_generations_list_envelope_golden() {
        let payload = CliEnvironmentGenerationsListPayload::new(
            command_payload("generations::list"),
            managed_env_detail(),
        )
        .with_request_tree(true);
        let value = serde_json::to_value(fixed_event(EventKind::CliEnvironmentGenerationsList(
            payload,
        )))
        .expect("event serializes");
        let mut expected =
            env_envelope_json("cli.environment.generations.list", "generations::list");
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
        let payload = CliBuildPayload::new(command_payload("build"), true, false);
        let value = serde_json::to_value(fixed_event(EventKind::CliBuild(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("build");
        payload_json
            .as_object_mut()
            .expect("payload object")
            .insert("has_expression_build".to_string(), json!(true));
        payload_json
            .as_object_mut()
            .expect("payload object")
            .insert("has_manifest_build".to_string(), json!(false));
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
        let payload = CliSearchPayload::new(command_payload("search"), "ripgrep".to_string());
        let value = serde_json::to_value(fixed_event(EventKind::CliSearch(payload)))
            .expect("event serializes");
        let mut payload_json = expected_payload_json("search");
        payload_json
            .as_object_mut()
            .expect("payload object")
            .insert("search_term".to_string(), json!("ripgrep"));
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
            shared_metadata().into_payload("install".to_string()),
            LifecycleFields {
                exit_code: 0,
                duration_ms: Some(1),
                error: None,
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
            shared_metadata(),
            connection,
        )
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
            error: None,
        })
        .expect("first completed record succeeds");
        hub.record_command_completed("install".to_string(), LifecycleFields {
            exit_code: 1,
            duration_ms: Some(200),
            error: Some(LifecycleError {
                kind: "env_not_found".to_string(),
                message: "environment not found".to_string(),
            }),
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
            error: None,
        })
        .unwrap();
        hub.flush(true).unwrap();
        hub.set_client(client_with_connection(&second_dir, second_conn));
        hub.record_command_completed("upgrade".to_string(), LifecycleFields {
            exit_code: 0,
            duration_ms: Some(1),
            error: None,
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

        hub.record_package_install("pkgA".to_string(), PackageOutcome::Success)
            .expect("record pkgA");
        hub.record_package_install("pkgB".to_string(), PackageOutcome::Success)
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

        // Simulating `Install::handle_error` calling
        // `record_package_install` for every attempted package on a
        // mid-pipeline failure.
        for package in ["pkgA", "nope"] {
            hub.record_package_install(package.to_string(), PackageOutcome::Failure)
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

        let env_detail = EnvDetail {
            env_kind: "path".to_string(),
            env_ref_or_name: "myenv".to_string(),
        };

        hub.record_environment_edit(env_detail.clone())
            .expect("eager emit");
        hub.record_environment_edit_with(env_detail, |p| p.with_edited_includes(true))
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
