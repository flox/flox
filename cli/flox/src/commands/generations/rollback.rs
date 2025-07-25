use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{GenerationsEnvironment, GenerationsExt};
use itertools::Itertools;
use tracing::{debug, instrument};

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::message;

/// Arguments for the `flox generations rollback` command
#[derive(Bpaf, Debug, Clone)]
pub struct Rollback {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Rollback {
    #[instrument(name = "rollback", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self.environment.to_concrete_environment(&flox)?;

        environment_subcommand_metric!("generations::rollback", env);
        let mut env: GenerationsEnvironment = env.try_into()?;

        debug!("determining previous generation");
        let metadata = env.generations_metadata()?;

        // (0, is the current active)
        let Some((previously_active_generation_id, _meta)) = metadata
            .generations
            .iter()
            .sorted_by_key(|(_id, meta)| meta.last_active)
            .nth(1)
        else {
            message::warning("No previous generation to rollback to.");
            return Ok(());
        };

        debug!(%previously_active_generation_id, "target generation determined, attempting rollback");
        env.switch_generation(&flox, *previously_active_generation_id)?;
        message::updated(format!(
            "Switched to generation {previously_active_generation_id}"
        ));

        Ok(())
    }
}
