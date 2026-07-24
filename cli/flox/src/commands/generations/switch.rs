use anyhow::Result;
use bpaf::Bpaf;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    GenerationId,
    GenerationsEnvironment,
    GenerationsExt,
};
use tracing::{debug, instrument};

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;

/// Arguments for the `flox generations switch` command
#[derive(Bpaf, Debug, Clone)]
pub struct Switch {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(positional("generation"))]
    target_generation: GenerationId,
}

impl Switch {
    #[instrument(name = "switch", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&mut flox, "Switch using")
            .await?;

        environment_subcommand_metric!("generations::switch", env);
        if let Err(err) =
            EventsHub::global().record_event(EventKind::CliEnvironmentGenerationsSwitch(
                CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &env)),
            ))
        {
            debug!(error = %err, "Failed to record v2 event");
        }
        let mut env: GenerationsEnvironment = env.try_into()?;

        let to = self.target_generation;
        debug!(%to, "target generation provided, attempting rollback");
        env.switch_generation(&flox, to)?;
        message::updated(format!("Switched to generation {to}"));
        Ok(())
    }
}
