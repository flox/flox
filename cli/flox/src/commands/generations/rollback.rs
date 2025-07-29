use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    AllGenerationsMetadata,
    GenerationId,
    GenerationsEnvironment,
    GenerationsExt,
    SingleGenerationMetadata,
};
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
        let Some((previously_active_generation_id, _meta)) =
            determine_previous_generation(&metadata)
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

/// "previous generation" currently means "previously active"
/// (as opposed to e.g. originating generation or generation N-1).
/// That implies that switching to the "previous generation",
/// i.e. rollback, returns at the original generation.
///
///   3 -rollback-> 2 -rollback-> 3
fn determine_previous_generation(
    metadata: &AllGenerationsMetadata,
) -> Option<(&GenerationId, &SingleGenerationMetadata)> {
    metadata
        .generations
        .iter()
        .sorted_by_key(|(_id, meta)| meta.last_active)
        .rev()
        .nth(1)
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::commands::generations::test_helpers::mock_generations;

    #[test]
    fn rollback_uses_previously_active() {
        // last created active
        let metadata = mock_generations(3.into());
        let Some((previous_generation, _metadata)) = determine_previous_generation(&metadata)
        else {
            panic!("expected to find previous generation")
        };

        assert_eq!(previous_generation, &2.into())
    }

    /// Use mock generations that were rolled back once from generation 3 -> genration 2.
    /// By our current definition of "previous generation" we expect another rollback
    /// to "roll forward" to generation 3, as thats the one previously active.
    #[test]
    fn rollback_rolls_back_to_newer_generation_if_previously_active() {
        // e.g. rolled back once 3->2
        let metadata = mock_generations(2.into());
        let Some((previous_generation, _metadata)) = determine_previous_generation(&metadata)
        else {
            panic!("expected to find previous generation")
        };

        assert_eq!(previous_generation, &3.into())
    }
}
