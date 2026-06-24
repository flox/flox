use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Result;
use tracing::{debug, trace};

use crate::client::EventsClient;
use crate::{
    CliEnvironmentActivatePayload,
    CliEnvironmentEditPayload,
    CliEnvironmentGenerationsListPayload,
    CliEnvironmentPublishPayload,
    EnvDetail,
    EventKind,
    PackageOutcome,
};

static EVENTS_HUB: LazyLock<EventsHub> = LazyLock::new(EventsHub::new);

/// Shared event client holder used by CLI call sites.
#[derive(Debug, Clone)]
pub struct EventsHub {
    client: Arc<Mutex<Option<EventsClient>>>,
    /// Sticky flag: set the first time `record_command_completed` runs
    /// against this hub, so a second call (e.g. when `activate.rs` emits
    /// before `command.exec()` and the dispatcher would otherwise emit
    /// again after `exec` returns an error) is a no-op. Reset whenever the
    /// installed client is replaced or cleared.
    completed_recorded: Arc<AtomicBool>,
}

impl EventsHub {
    pub fn global() -> &'static Self {
        &EVENTS_HUB
    }

    pub fn new() -> Self {
        Self {
            client: Arc::new(Mutex::new(None)),
            completed_recorded: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn set_client(&self, new_client: EventsClient) -> Option<EventsClient> {
        self.completed_recorded.store(false, Ordering::SeqCst);
        self.with_client(|client| client.replace(new_client))
    }

    pub fn clear_client(&self) -> Option<EventsClient> {
        self.completed_recorded.store(false, Ordering::SeqCst);
        self.with_client(Option::take)
    }

    pub fn flush(&self, force: bool) -> Result<()> {
        self.with_client(|client| {
            if let Some(client) = client {
                client.flush(force)
            } else {
                trace!("No v2 events client configured, skipping flush");
                Ok(())
            }
        })
    }

    pub fn record_event(&self, kind: EventKind) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping record");
                return Ok(());
            };

            client.record_event(kind)
        })
    }

    /// Record a `cli.command_run` event for `subcommand`. No-op when no
    /// client is installed.
    pub fn record_command_run(&self, subcommand: String) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping command_run record");
                return Ok(());
            };
            client.record_command_run(subcommand)
        })
    }

    /// Record a `cli.command_completed` event for `subcommand`. No-op when
    /// no client is installed. Subsequent calls against the same client
    /// install are no-ops so the dispatcher and the `activate.rs` pre-exec
    /// path cannot race-emit twice for one invocation.
    pub fn record_command_completed(&self, subcommand: String) -> Result<()> {
        if self.completed_recorded.swap(true, Ordering::SeqCst) {
            debug!("command_completed already recorded for this client install, skipping");
            return Ok(());
        }
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping command_completed record");
                return Ok(());
            };
            client.record_command_completed(subcommand)
        })
    }

    /// Record a `cli.environment.activate` event from an already-built
    /// payload. Most call sites should prefer
    /// [`EventsHub::record_environment_activate_with`], which builds the
    /// payload and applies the activate extras in a single call.
    /// No-op when no client is installed.
    pub fn record_environment_activate(
        &self,
        payload: CliEnvironmentActivatePayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.activate record");
                return Ok(());
            };
            client.record_environment_activate(payload)
        })
    }

    /// Record a `cli.environment.activate` event in one call: builds the
    /// payload from the installed client's shared metadata, applies
    /// `extras` to populate the activate-specific Optional fields, and
    /// records it. When no client is installed the call short-circuits
    /// without invoking `extras` — call sites do not need to write
    /// `if let Some(payload) = …`.
    pub fn record_environment_activate_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentActivatePayload) -> CliEnvironmentActivatePayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.activate record");
                return Ok(());
            };
            client.record_environment_activate_with(env_detail, extras)
        })
    }

    /// Record a `cli.environment.push` event with the supplied env detail.
    /// No-op when no client is installed.
    pub fn record_environment_push(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.push record");
                return Ok(());
            };
            client.record_environment_push(env_detail)
        })
    }

    /// Record a `cli.environment.pull` event with the supplied env detail.
    /// No-op when no client is installed.
    pub fn record_environment_pull(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.pull record");
                return Ok(());
            };
            client.record_environment_pull(env_detail)
        })
    }

    /// Record a `cli.package.install` event for one package + its outcome.
    /// No-op when no client is installed.
    pub fn record_package_install(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping package.install record");
                return Ok(());
            };
            client.record_package_install(package, outcome)
        })
    }

    /// Record a `cli.package.upgrade` event for one package + its outcome.
    /// No-op when no client is installed.
    pub fn record_package_upgrade(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping package.upgrade record");
                return Ok(());
            };
            client.record_package_upgrade(package, outcome)
        })
    }

    /// Record a `cli.package.uninstall` event for one package + its outcome.
    /// No-op when no client is installed.
    pub fn record_package_uninstall(&self, package: String, outcome: PackageOutcome) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping package.uninstall record");
                return Ok(());
            };
            client.record_package_uninstall(package, outcome)
        })
    }

    /// Record a `cli.environment.containerize` event. No-op when no
    /// client is installed.
    pub fn record_environment_containerize(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.containerize record");
                return Ok(());
            };
            client.record_environment_containerize(env_detail)
        })
    }

    /// Record a `cli.environment.delete` event. No-op when no client
    /// is installed.
    pub fn record_environment_delete(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.delete record");
                return Ok(());
            };
            client.record_environment_delete(env_detail)
        })
    }

    /// Record a `cli.environment.include.upgrade` event. No-op when
    /// no client is installed.
    pub fn record_environment_include_upgrade(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.include.upgrade record"
                );
                return Ok(());
            };
            client.record_environment_include_upgrade(env_detail)
        })
    }

    /// Record a `cli.environment.install` event (env-detail row). The
    /// per-package detail rides on `cli.package.install`. No-op when
    /// no client is installed.
    pub fn record_environment_install(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.install record");
                return Ok(());
            };
            client.record_environment_install(env_detail)
        })
    }

    /// Record a `cli.environment.list` event. No-op when no client is
    /// installed.
    pub fn record_environment_list(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.list record");
                return Ok(());
            };
            client.record_environment_list(env_detail)
        })
    }

    /// Record a `cli.environment.uninstall` event (env-detail row).
    /// The per-package detail rides on `cli.package.uninstall`. No-op
    /// when no client is installed.
    pub fn record_environment_uninstall(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.uninstall record");
                return Ok(());
            };
            client.record_environment_uninstall(env_detail)
        })
    }

    /// Record a `cli.environment.upgrade` event (env-detail row). The
    /// per-package detail rides on `cli.package.upgrade`. No-op when
    /// no client is installed.
    pub fn record_environment_upgrade(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.upgrade record");
                return Ok(());
            };
            client.record_environment_upgrade(env_detail)
        })
    }

    /// Record a `cli.environment.services.start` event. No-op when no
    /// client is installed.
    pub fn record_environment_services_start(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.services.start record"
                );
                return Ok(());
            };
            client.record_environment_services_start(env_detail)
        })
    }

    /// Record a `cli.environment.services.stop` event. No-op when no
    /// client is installed.
    pub fn record_environment_services_stop(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.services.stop record");
                return Ok(());
            };
            client.record_environment_services_stop(env_detail)
        })
    }

    /// Record a `cli.environment.services.restart` event. No-op when
    /// no client is installed.
    pub fn record_environment_services_restart(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.services.restart record"
                );
                return Ok(());
            };
            client.record_environment_services_restart(env_detail)
        })
    }

    /// Record a `cli.environment.services.status` event. No-op when
    /// no client is installed.
    pub fn record_environment_services_status(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.services.status record"
                );
                return Ok(());
            };
            client.record_environment_services_status(env_detail)
        })
    }

    /// Record a `cli.environment.services.logs` event. No-op when no
    /// client is installed.
    pub fn record_environment_services_logs(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.services.logs record");
                return Ok(());
            };
            client.record_environment_services_logs(env_detail)
        })
    }

    /// Record a `cli.environment.services.persist` event. No-op when
    /// no client is installed.
    pub fn record_environment_services_persist(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.services.persist record"
                );
                return Ok(());
            };
            client.record_environment_services_persist(env_detail)
        })
    }

    /// Record a `cli.environment.generations.history` event. No-op
    /// when no client is installed.
    pub fn record_environment_generations_history(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.generations.history record"
                );
                return Ok(());
            };
            client.record_environment_generations_history(env_detail)
        })
    }

    /// Record a `cli.environment.generations.rollback` event. No-op
    /// when no client is installed.
    pub fn record_environment_generations_rollback(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.generations.rollback record"
                );
                return Ok(());
            };
            client.record_environment_generations_rollback(env_detail)
        })
    }

    /// Record a `cli.environment.generations.switch` event. No-op
    /// when no client is installed.
    pub fn record_environment_generations_switch(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.generations.switch record"
                );
                return Ok(());
            };
            client.record_environment_generations_switch(env_detail)
        })
    }

    /// Record a `cli.environment.edit` event with just env detail
    /// (eager call site). No-op when no client is installed.
    pub fn record_environment_edit(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.edit record");
                return Ok(());
            };
            client.record_environment_edit(env_detail)
        })
    }

    /// Record a `cli.environment.edit` event populated by `extras`
    /// (result-known call site). No-op when no client is installed —
    /// the `extras` closure is not invoked in that case.
    pub fn record_environment_edit_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentEditPayload) -> CliEnvironmentEditPayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.edit record");
                return Ok(());
            };
            client.record_environment_edit_with(env_detail, extras)
        })
    }

    /// Record a `cli.environment.publish` event with just env detail
    /// (eager call site). No-op when no client is installed.
    pub fn record_environment_publish(&self, env_detail: EnvDetail) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.publish record");
                return Ok(());
            };
            client.record_environment_publish(env_detail)
        })
    }

    /// Record a `cli.environment.publish` event populated by `extras`
    /// (result-known call site). No-op when no client is installed.
    pub fn record_environment_publish_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(CliEnvironmentPublishPayload) -> CliEnvironmentPublishPayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping environment.publish record");
                return Ok(());
            };
            client.record_environment_publish_with(env_detail, extras)
        })
    }

    /// Record a `cli.environment.generations.list` event populated by
    /// `extras`. No-op when no client is installed.
    pub fn record_environment_generations_list_with(
        &self,
        env_detail: EnvDetail,
        extras: impl FnOnce(
            CliEnvironmentGenerationsListPayload,
        ) -> CliEnvironmentGenerationsListPayload,
    ) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!(
                    "No v2 events client configured, skipping environment.generations.list record"
                );
                return Ok(());
            };
            client.record_environment_generations_list_with(env_detail, extras)
        })
    }

    /// Record a `cli.build` event carrying build-kind detection
    /// flags. No-op when no client is installed.
    pub fn record_build(&self, has_expression_build: bool, has_manifest_build: bool) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping build record");
                return Ok(());
            };
            client.record_build(has_expression_build, has_manifest_build)
        })
    }

    /// Record a `cli.search` event carrying the search term. No-op
    /// when no client is installed.
    pub fn record_search(&self, search_term: String) -> Result<()> {
        self.with_client(|client| {
            let Some(client) = client else {
                trace!("No v2 events client configured, skipping search record");
                return Ok(());
            };
            client.record_search(search_term)
        })
    }

    fn with_client<T>(&self, f: impl FnOnce(&mut Option<EventsClient>) -> T) -> T {
        let mut client = self
            .client
            .lock()
            .expect("v2 events client mutex panicked on another thread");
        f(&mut client)
    }
}

impl Default for EventsHub {
    fn default() -> Self {
        Self::new()
    }
}
