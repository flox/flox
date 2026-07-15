use std::path::{Path, PathBuf};

use anyhow::Result;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

use crate::buffer::EventsBuffer;
use crate::connection::{EventsConnection, EventsConnectionV2};
use crate::{
    CliBuildPayload,
    CliCommandCompletedPayload,
    CliCommandRunPayload,
    CliEnvironmentActivatePayload,
    CliEnvironmentContainerizePayload,
    CliEnvironmentDeletePayload,
    CliEnvironmentEditPayload,
    CliEnvironmentGenerationsHistoryPayload,
    CliEnvironmentGenerationsListPayload,
    CliEnvironmentGenerationsRollbackPayload,
    CliEnvironmentGenerationsSwitchPayload,
    CliEnvironmentIncludeUpgradePayload,
    CliEnvironmentInstallPayload,
    CliEnvironmentListPayload,
    CliEnvironmentPublishPayload,
    CliEnvironmentPullPayload,
    CliEnvironmentPushPayload,
    CliEnvironmentServicesLogsPayload,
    CliEnvironmentServicesPersistPayload,
    CliEnvironmentServicesRestartPayload,
    CliEnvironmentServicesStartPayload,
    CliEnvironmentServicesStatusPayload,
    CliEnvironmentServicesStopPayload,
    CliEnvironmentUninstallPayload,
    CliEnvironmentUpgradePayload,
    CliPackageInstallPayload,
    CliPackageUninstallPayload,
    CliPackageUpgradePayload,
    CliSearchPayload,
    EnvDetail,
    Event,
    EventKind,
    LifecycleFields,
    PackageOutcome,
    SharedMetadataTemplate,
};

const DEFAULT_BUFFER_EXPIRY: Duration = Duration::minutes(2);
pub const BATCH_SIZE: usize = 100;

/// Client that stamps v2 event metadata, buffers events, and flushes
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
        let connection = EventsConnectionV2::new(endpoint_url, api_key);
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

    /// Record a `cli.command_completed` event carrying the dispatch
    /// lifecycle fields.
    pub fn record_command_completed(
        &self,
        subcommand: String,
        lifecycle: LifecycleFields,
    ) -> Result<()> {
        let payload = CliCommandCompletedPayload::new(
            self.shared_metadata.into_payload(subcommand),
            lifecycle,
        );
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

    /// Record a `cli.environment.containerize` event with env detail.
    pub fn record_environment_containerize(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentContainerizePayload::new(
            self.shared_metadata
                .into_payload("containerize".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentContainerize(payload))
    }

    /// Record a `cli.environment.delete` event with env detail.
    pub fn record_environment_delete(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentDeletePayload::new(
            self.shared_metadata.into_payload("delete".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentDelete(payload))
    }

    /// Record a `cli.environment.include.upgrade` event with env
    /// detail.
    pub fn record_environment_include_upgrade(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentIncludeUpgradePayload::new(
            self.shared_metadata
                .into_payload("include::upgrade".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentIncludeUpgrade(payload))
    }

    /// Record a `cli.environment.install` event with env detail.
    /// The per-package detail rides on `cli.package.install`.
    pub fn record_environment_install(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentInstallPayload::new(
            self.shared_metadata.into_payload("install".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentInstall(payload))
    }

    /// Record a `cli.environment.list` event with env detail.
    pub fn record_environment_list(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentListPayload::new(
            self.shared_metadata.into_payload("list".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentList(payload))
    }

    /// Record a `cli.environment.uninstall` event with env detail.
    /// The per-package detail rides on `cli.package.uninstall`.
    pub fn record_environment_uninstall(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentUninstallPayload::new(
            self.shared_metadata.into_payload("uninstall".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentUninstall(payload))
    }

    /// Record a `cli.environment.upgrade` event with env detail.
    /// The per-package detail rides on `cli.package.upgrade`.
    pub fn record_environment_upgrade(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentUpgradePayload::new(
            self.shared_metadata.into_payload("upgrade".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentUpgrade(payload))
    }

    /// Record a `cli.environment.services.start` event with env detail.
    pub fn record_environment_services_start(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesStartPayload::new(
            self.shared_metadata
                .into_payload("services::start".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesStart(payload))
    }

    /// Record a `cli.environment.services.stop` event with env detail.
    pub fn record_environment_services_stop(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesStopPayload::new(
            self.shared_metadata
                .into_payload("services::stop".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesStop(payload))
    }

    /// Record a `cli.environment.services.restart` event with env detail.
    pub fn record_environment_services_restart(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesRestartPayload::new(
            self.shared_metadata
                .into_payload("services::restart".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesRestart(payload))
    }

    /// Record a `cli.environment.services.status` event with env detail.
    pub fn record_environment_services_status(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesStatusPayload::new(
            self.shared_metadata
                .into_payload("services::status".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesStatus(payload))
    }

    /// Record a `cli.environment.services.logs` event with env detail.
    pub fn record_environment_services_logs(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesLogsPayload::new(
            self.shared_metadata
                .into_payload("services::logs".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesLogs(payload))
    }

    /// Record a `cli.environment.services.persist` event with env detail.
    pub fn record_environment_services_persist(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentServicesPersistPayload::new(
            self.shared_metadata
                .into_payload("services::persist".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentServicesPersist(payload))
    }

    /// Record a `cli.environment.generations.history` event with env detail.
    pub fn record_environment_generations_history(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentGenerationsHistoryPayload::new(
            self.shared_metadata
                .into_payload("generations::history".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentGenerationsHistory(payload))
    }

    /// Record a `cli.environment.generations.rollback` event with env detail.
    pub fn record_environment_generations_rollback(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentGenerationsRollbackPayload::new(
            self.shared_metadata
                .into_payload("generations::rollback".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentGenerationsRollback(payload))
    }

    /// Record a `cli.environment.generations.switch` event with env detail.
    pub fn record_environment_generations_switch(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentGenerationsSwitchPayload::new(
            self.shared_metadata
                .into_payload("generations::switch".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentGenerationsSwitch(payload))
    }

    /// Record a `cli.environment.edit` event with just env detail.
    /// Used at the eager call site (before the edit operation runs);
    /// the `edited_includes` field is left unpopulated and the
    /// result-known site emits a second event via
    /// [`Self::record_environment_edit_with`].
    pub fn record_environment_edit(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentEditPayload::new(
            self.shared_metadata.into_payload("edit".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentEdit(payload))
    }

    /// Record a `cli.environment.edit` event populated by `extras`.
    /// Builds the payload from the installed shared metadata + env
    /// detail and lets the call site set `edited_includes` via the
    /// builder closure.
    pub fn record_environment_edit_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentEditPayload) -> CliEnvironmentEditPayload,
    ) -> Result<()> {
        let payload = CliEnvironmentEditPayload::new(
            self.shared_metadata.into_payload("edit".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentEdit(extras(payload)))
    }

    /// Record a `cli.environment.publish` event with just env detail.
    pub fn record_environment_publish(&self, env_detail: EnvDetail) -> Result<()> {
        let payload = CliEnvironmentPublishPayload::new(
            self.shared_metadata.into_payload("publish".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentPublish(payload))
    }

    /// Record a `cli.environment.publish` event populated by `extras`.
    pub fn record_environment_publish_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentPublishPayload) -> CliEnvironmentPublishPayload,
    ) -> Result<()> {
        let payload = CliEnvironmentPublishPayload::new(
            self.shared_metadata.into_payload("publish".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentPublish(extras(payload)))
    }

    /// Record a `cli.environment.generations.list` event populated by
    /// `extras`. Single call site (no separate eager-emit form).
    pub fn record_environment_generations_list_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(
            CliEnvironmentGenerationsListPayload,
        ) -> CliEnvironmentGenerationsListPayload,
    ) -> Result<()> {
        let payload = CliEnvironmentGenerationsListPayload::new(
            self.shared_metadata
                .into_payload("generations::list".to_string()),
            env_detail,
        );
        self.record_event(EventKind::CliEnvironmentGenerationsList(extras(payload)))
    }

    /// Record a `cli.build` event carrying build-kind detection flags.
    pub fn record_build(&self, has_expression_build: bool, has_manifest_build: bool) -> Result<()> {
        let payload = CliBuildPayload::new(
            self.shared_metadata.into_payload("build".to_string()),
            has_expression_build,
            has_manifest_build,
        );
        self.record_event(EventKind::CliBuild(payload))
    }

    /// Record a `cli.search` event carrying the user-supplied search
    /// term verbatim (D3, 2026-05-30).
    pub fn record_search(&self, search_term: String) -> Result<()> {
        let payload = CliSearchPayload::new(
            self.shared_metadata.into_payload("search".to_string()),
            search_term,
        );
        self.record_event(EventKind::CliSearch(payload))
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
