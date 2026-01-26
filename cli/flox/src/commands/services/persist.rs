use std::env::consts::OS;
use std::fs::File;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::data::environment_ref::ActivateEnvironmentRef;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::manifest::typed::{Inner, ServiceDescriptor};
use flox_rust_sdk::providers::services::systemd::render_systemd_unit_file;
use tracing::instrument;
use xdg::BaseDirectories;

use crate::commands::services::{ServicesEnvironment, guard_service_commands_available};
use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
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
    pub async fn handle(self, flox: Flox) -> Result<()> {
        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        environment_subcommand_metric!("services::persist", env.environment);
        guard_service_commands_available(&env, &flox.system)?;

        let services_for_system = env.manifest.services.copy_for_system(&flox.system);
        let services_to_persist: Vec<_> = if self.names.is_empty() {
            services_for_system.inner().iter().collect()
        } else {
            self.names
                .iter()
                .map(|name| {
                    let descriptor = services_for_system.inner().get(name);
                    let exists_for_other_systems = env.manifest.services.inner().contains_key(name);
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
    }

    message::info("To apply the changes, run: 'systemctl --user daemon-reload'");

    Ok(())
}
