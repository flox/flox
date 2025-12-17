use anyhow::{Result, bail};
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
use crate::utils::{bail_on_v2_manifest_without_feature_flag, message};

/// Arguments for the `flox generations rollback` command
#[derive(Bpaf, Debug, Clone)]
pub struct Rollback {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Rollback {
    #[instrument(name = "rollback", skip_all)]
    pub fn handle(self, flox: Flox) -> Result<()> {
        // TODO(zmitchell, 2025-20-12): `detect_concrete_environment` will prompt
        // which environment to use even when one of the choices is a path
        // environment (which fails when selected). We could be smarter about
        // this in the future.
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Rollback using")?;
        bail_on_v2_manifest_without_feature_flag(&flox, &env)?;

        environment_subcommand_metric!("generations::rollback", env);
        let mut env: GenerationsEnvironment = env.try_into()?;

        debug!("determining previous generation");
        let metadata = env.generations_metadata()?;

        // (0, is the current active)
        let Some((previously_active_generation_id, _meta)) =
            determine_previous_generation(&metadata)
        else {
            bail!("No previous generation to rollback to.");
        };

        debug!(%previously_active_generation_id, "target generation determined, attempting rollback");
        env.switch_generation(&flox, previously_active_generation_id)?;
        message::updated(format!(
            "Switched to generation {previously_active_generation_id}"
        ));

        Ok(())
    }
}

/// "previous generation" currently means "previously live"
/// (as opposed to e.g. originating generation or generation N-1).
/// That implies that switching to the "previous generation",
/// i.e. rollback, returns at the original generation.
///
///   3 -rollback-> 2 -rollback-> 3
fn determine_previous_generation(
    metadata: &AllGenerationsMetadata,
) -> Option<(GenerationId, SingleGenerationMetadata)> {
    let generations = metadata.generations();
    // Can't rollback if there's only 1 generation
    if generations.len() < 2 {
        return None;
    }
    // The current live generation has last_live = None, so it will be at the
    // front
    generations
        .into_iter()
        .sorted_by_key(|(_id, meta)| meta.last_live)
        .next_back()
}

#[cfg(test)]
mod tests {

    use flox_rust_sdk::models::environment::generations::test_helpers::{
        default_add_generation_options,
        default_switch_generation_options,
    };

    use super::*;

    #[test]
    fn rollback_uses_previously_active() {
        // last created active
        let mut metadata = AllGenerationsMetadata::default();
        metadata.add_generation(default_add_generation_options());
        let (expected_prev_id, ..) = metadata.add_generation(default_add_generation_options());
        metadata.add_generation(default_add_generation_options());

        let Some((previous_generation, _metadata)) = determine_previous_generation(&metadata)
        else {
            panic!("expected to find previous generation")
        };

        assert_eq!(previous_generation, expected_prev_id)
    }

    /// Use mock generations that were rolled back once from generation 3 -> genration 2.
    /// By our current definition of "previous generation" we expect another rollback
    /// to "roll forward" to generation 3, as thats the one previously live.
    #[test]
    fn rollback_rolls_back_to_newer_generation_if_previously_live() {
        // e.g. rolled back once 3->2
        // last created live
        let mut metadata = AllGenerationsMetadata::default();
        metadata.add_generation(default_add_generation_options());
        metadata.add_generation(default_add_generation_options());
        let (third_generation, ..) = metadata.add_generation(default_add_generation_options());
        metadata
            .switch_generation(default_switch_generation_options(
                determine_previous_generation(&metadata).unwrap().0,
            ))
            .unwrap();

        let Some((previous_generation, _metadata)) = determine_previous_generation(&metadata)
        else {
            panic!("expected to find previous generation")
        };

        assert_eq!(previous_generation, third_generation)
    }

    #[test]
    fn no_previous_generation_for_single_generation() {
        let mut metadata = AllGenerationsMetadata::default();
        metadata.add_generation(default_add_generation_options());
        assert!(determine_previous_generation(&metadata).is_none());
    }
}
