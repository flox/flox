use std::path::{Path, PathBuf};

use anyhow::Result;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::buffer::EventsBuffer;
use crate::connection::{CanonicalEventsConnection, EventsConnection};
use crate::{
    CliCommandCompletedPayload,
    CliCommandRunPayload,
    CliEnvironmentActivatePayload,
    CliEnvironmentPullPayload,
    CliEnvironmentPushPayload,
    CliPackageInstallPayload,
    CliPackageUninstallPayload,
    CliPackageUpgradePayload,
    EnvDetail,
    Event,
    EventKind,
    PackageOutcome,
    SharedMetadataTemplate,
};

const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);
pub const BATCH_SIZE: usize = 100;

/// Client that stamps canonical event metadata, buffers events, and flushes
/// them through an [`EventsConnection`].
///
/// The connection owns the endpoint URL and credential; the client itself
/// holds the per-invocation identity (`device_id`, `invocation_id`) and the
/// static shared metadata template stamped onto every command event payload.
#[derive(Debug)]
pub struct EventsClient {
    pub device_id: Uuid,
    pub data_dir: PathBuf,
    pub invocation_id: Uuid,
    pub max_age: Duration,
    pub connection: Box<dyn EventsConnection>,
    shared_metadata: SharedMetadataTemplate,
}

impl EventsClient {
    pub fn new(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        endpoint_url: impl Into<String>,
        api_key: impl Into<String>,
        invocation_id: Uuid,
        shared_metadata: SharedMetadataTemplate,
    ) -> Self {
        let connection = CanonicalEventsConnection::new(endpoint_url, api_key);
        Self::new_with_connection(
            device_id,
            data_dir,
            invocation_id,
            shared_metadata,
            connection,
        )
    }

    pub fn new_with_connection(
        device_id: Uuid,
        data_dir: impl AsRef<Path>,
        invocation_id: Uuid,
        shared_metadata: SharedMetadataTemplate,
        connection: impl EventsConnection + 'static,
    ) -> Self {
        Self {
            device_id,
            data_dir: data_dir.as_ref().to_path_buf(),
            invocation_id,
            max_age: DEFAULT_BUFFER_EXPIRY,
            connection: connection.boxed(),
            shared_metadata,
        }
    }

    /// Record a `cli.command_run` event for `subcommand`.
    pub fn record_command_run(&self, subcommand: String) -> Result<()> {
        let payload = CliCommandRunPayload::new(self.shared_metadata.into_payload(subcommand));
        self.record_event(EventKind::CliCommandRun(payload))
    }

    /// Record a `cli.command_completed` event for `subcommand`.
    pub fn record_command_completed(&self, subcommand: String) -> Result<()> {
        let payload =
            CliCommandCompletedPayload::new(self.shared_metadata.into_payload(subcommand));
        self.record_event(EventKind::CliCommandCompleted(payload))
    }

    /// Record a `cli.environment.activate` event with the supplied env
    /// detail. The caller is expected to populate any of the activate-
    /// specific extras (`start_services`, `mode`, `has_includes`,
    /// `lockfile_version`, `shell`) via the builder methods on the payload
    /// before this is called, e.g. via [`build_environment_activate_payload`].
    pub fn record_environment_activate(
        &self,
        payload: CliEnvironmentActivatePayload,
    ) -> Result<()> {
        self.record_event(EventKind::CliEnvironmentActivate(payload))
    }

    /// Build a `cli.environment.activate` payload using the stored shared
    /// metadata (with `subcommand = "activate"`) and the supplied env
    /// detail. The result has every activate extra `None`; the caller chains
    /// `with_*` builder methods on the returned value before passing to
    /// [`record_environment_activate`].
    pub fn build_environment_activate_payload(
        &self,
        env_detail: EnvDetail,
    ) -> CliEnvironmentActivatePayload {
        CliEnvironmentActivatePayload::new(
            self.shared_metadata.into_payload("activate".to_string()),
            env_detail,
        )
    }

    /// Convenience wrapper: build a `cli.environment.activate` payload, apply
    /// `extras` to populate the activate-specific Optional fields (e.g.
    /// `|p| p.with_shell(shell.to_string())`), and record the event in one
    /// call. The call site never sees the `None`-client branch.
    pub fn record_environment_activate_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentActivatePayload) -> CliEnvironmentActivatePayload,
    ) -> Result<()> {
        let payload = extras(self.build_environment_activate_payload(env_detail));
        self.record_environment_activate(payload)
    }

    /// Record a `cli.environment.push` event with the supplied env detail.
    pub fn record_environment_push(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentPushPayload::new(
            self.shared_metadata.into_payload("push".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentPush(payload))
    }

    /// Record a `cli.environment.pull` event with the supplied env detail.
    pub fn record_environment_pull(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentPullPayload::new(
            self.shared_metadata.into_payload("pull".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentPull(payload))
    }

    /// Record a `cli.package.install` event for one package + its outcome.
    pub fn record_package_install(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        let payload = CliPackageInstallPayload::new(
            self.shared_metadata.into_payload("install".to_string()),
            package,
            outcome,
        );
        self.record_event(EventKind::CliPackageInstall(payload))
    }

    /// Record a `cli.package.upgrade` event for one package + its outcome.
    pub fn record_package_upgrade(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        let payload = CliPackageUpgradePayload::new(
            self.shared_metadata.into_payload("upgrade".to_string()),
            package,
            outcome,
        );
        self.record_event(EventKind::CliPackageUpgrade(payload))
    }

    /// Record a `cli.package.uninstall` event for one package + its outcome.
    pub fn record_package_uninstall(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        let payload = CliPackageUninstallPayload::new(
            self.shared_metadata.into_payload("uninstall".to_string()),
            package,
            outcome,
        );
        self.record_event(EventKind::CliPackageUninstall(payload))
    }

    pub fn record_event(&self, kind: impl Into<EventKind>) -> Result<()> {
        let event = Event {
            event_id: Uuid::new_v4(),
            event_timestamp: OffsetDateTime::now_utc(),
            source: "cli",
            invocation_id: self.invocation_id,
            device_id: self.device_id,
            auth_subject: None,
            kind: kind.into(),
        };

        let mut events_buffer = EventsBuffer::read(&self.data_dir)?;
        events_buffer.push(event)?;
        Ok(())
    }

    pub fn flush(&mut self, force: bool) -> Result<()> {
        let mut events = EventsBuffer::read(&self.data_dir)?;
        if !events.is_expired(self.max_age) && !force {
            return Ok(());
        }

        while !events.is_empty() {
            let batch_size = events.batch_size(BATCH_SIZE);
            {
                let batch: Vec<&Event> = events.iter().take(batch_size).collect();
                self.connection.send(batch)?;
            }

            events.drain_sent(batch_size);
            events.overwrite_file()?;
        }

        Ok(())
    }
}
