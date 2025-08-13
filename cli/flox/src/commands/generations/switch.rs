use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::generations::{
    GenerationId,
    GenerationsEnvironment,
    GenerationsExt,
};
use tracing::{debug, instrument};

use crate::commands::{EnvironmentSelect, environment_select};
use crate::environment_subcommand_metric;
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
    pub fn handle(self, flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Switch using")?;

        environment_subcommand_metric!("generations::switch", env);
        let mut env: GenerationsEnvironment = env.try_into()?;

        let to = self.target_generation;
        debug!(%to, "target generation provided, attempting rollback");
        env.switch_generation(&flox, to)?;
        message::updated(format!("Switched to generation {to}"));
        return Ok(());
    }
}
