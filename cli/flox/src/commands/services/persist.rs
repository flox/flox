use std::env::consts::OS;
use std::fs::File;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::data::environment_ref::ActivateEnvironmentRef;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_manifest::interfaces::AsLatestSchema;
use flox_manifest::parsed::Inner;
use flox_manifest::parsed::latest::ServiceDescriptor;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::systemd::render_systemd_unit_file;
use indoc::formatdoc;
use tracing::{debug, instrument};
use xdg::BaseDirectories;

use crate::commands::services::{ServicesEnvironment, guard_service_commands_available};
use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;

// TODO: Allow output directory to be configurable? But consider whether it
//       would work the same for another backend like launchd
#[derive(Bpaf, Debug, Clone)]
pub struct Persist {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to persist
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Persist {
    #[instrument(name = "persist", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let env =
            ServicesEnvironment::from_environment_selection(&mut flox, &self.environment).await?;
        environment_subcommand_metric!("services::persist", env.environment);
        if let Err(err) =
            EventsHub::global().record_event(EventKind::CliEnvironmentServicesPersist(
                CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &env.environment)),
            ))
        {
            debug!(error = %err, "Failed to record v2 event");
        }
        guard_service_commands_available(&env, &flox.system)?;

        let manifest_services = &env.manifest.as_latest_schema().services;
        let services_for_system = manifest_services.copy_for_system(&flox.system);
        let services_to_persist: Vec<_> = if self.names.is_empty() {
            services_for_system.inner().iter().collect()
        } else {
            self.names
                .iter()
                .map(|name| {
                    let descriptor = services_for_system.inner().get(name);
                    let exists_for_other_systems = manifest_services.inner().contains_key(name);
                    match (descriptor, exists_for_other_systems) {
                        (Some(descriptor), _) => Ok((name, descriptor)),
                        (None, true) => Err(super::service_not_available_on_system_error(
                            name,
                            &flox.system,
                        )
                        .into()),
                        (None, false) => Err(super::service_does_not_exist_error(name).into()),
                    }
                })
                .collect::<Result<Vec<_>>>()?
        };

        if services_to_persist.is_empty() {
            message::warning("No services to persist for this system");
            return Ok(());
        }

        let env_ref = ActivateEnvironmentRef::try_from(&env.environment)?;

        // TODO: Detect working systemd install rather than OS?
        match OS {
            "linux" => persist_systemd(env_ref, services_to_persist),
            _ => bail!("This command is currently only supported on Linux systems."),
        }
    }
}

/// A manifest field the generated systemd unit does not carry, with the
/// sentence telling the reader what to do about it.
///
/// `remedy` is `None` where the unit types Flox renders have no counterpart at
/// all, so that the message can say so rather than send the reader looking for
/// a key that does not exist.
struct NotCarried {
    field: &'static str,
    remedy: Option<String>,
}

/// Names the fields of `descriptor` that the generated systemd unit does not
/// carry.
///
/// A unit file is rendered from `command`, `vars`, `is-daemon` and
/// `shutdown.command`. The service orchestration fields are honored by
/// process-compose, which backs `flox activate` and `flox services`, and Flox
/// derives no systemd counterpart for them: `depends-on` would have to name
/// units that may not have been persisted, and ordering and the stop timeout
/// are reached through the `systemd` passthrough instead.
fn fields_not_carried_into_unit(
    service_name: &str,
    descriptor: &ServiceDescriptor,
) -> Vec<NotCarried> {
    // Destructuring here forces a field added to the descriptor to be
    // classified rather than silently joining the carried set. `systems` is
    // applied before this point, and `systemd` is the passthrough the remedies
    // point at.
    let ServiceDescriptor {
        command: _,
        vars: _,
        is_daemon: _,
        shutdown,
        depends_on,
        systemd: _,
        systems: _,
    } = descriptor;
    let passthrough = format!("[services.{service_name}.systemd]");

    let mut fields = Vec::new();
    if depends_on.is_some() {
        fields.push(NotCarried {
            field: "depends-on",
            remedy: Some(format!(
                "Set 'unit.after' or 'unit.requires' under '{passthrough}' to order the unit."
            )),
        });
    }
    let Some(shutdown) = shutdown.as_ref() else {
        return fields;
    };
    if shutdown.timeout_seconds.is_some() {
        fields.push(NotCarried {
            field: "shutdown.timeout-seconds",
            remedy: Some(format!(
                "Set 'service.timeout_stop_sec' under '{passthrough}' to bound the stop."
            )),
        });
    }
    if shutdown.signal.is_some() {
        // `systemd::unit::Service` has no kill-signal field and denies unknown
        // keys, so pointing at the passthrough here would send the reader into
        // a parse error.
        fields.push(NotCarried {
            field: "shutdown.signal",
            remedy: None,
        });
    }
    fields
}

fn persist_systemd(
    env_ref: ActivateEnvironmentRef,
    services_to_persist: Vec<(&String, &ServiceDescriptor)>,
) -> Result<()> {
    let systemd_dirs = BaseDirectories::with_prefix("systemd/user");

    for (service_name, service_descriptor) in services_to_persist {
        let unit_filename = format!("{}.service", service_name);
        let unit_path = systemd_dirs.place_config_file(&unit_filename)?;

        let mut output_file = File::create(&unit_path)?;
        render_systemd_unit_file(&env_ref, service_descriptor, &mut output_file)?;

        // TODO: Differentiate between file creation and update?
        message::updated(format!(
            "Wrote {} to {}",
            unit_filename,
            unit_path.display()
        ));

        // After the line naming the unit, so that the warning reads as being
        // about the unit just written rather than the one before it.
        let not_carried = fields_not_carried_into_unit(service_name, service_descriptor);
        if !not_carried.is_empty() {
            let names = not_carried
                .iter()
                .map(|entry| format!("'{}'", entry.field))
                .collect::<Vec<_>>()
                .join(", ");
            let remedies = not_carried
                .iter()
                .map(|entry| match &entry.remedy {
                    Some(remedy) => remedy.clone(),
                    None => format!(
                        "There is no systemd equivalent for '{}', so the unit stops with the default signal.",
                        entry.field
                    ),
                })
                .collect::<Vec<_>>()
                .join("\n");
            message::warning(formatdoc! {"
                Service '{service_name}' sets options that '{unit_filename}' does not carry: {names}.
                {remedies}
            "});
        }
    }

    message::info("To apply the changes, run: 'systemctl --user daemon-reload'");

    Ok(())
}
