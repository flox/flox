use bpaf::Bpaf;
use flox_core::activations::activation_state_dir_path;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;

use super::{EnvironmentSelect, environment_select};

#[derive(Debug, Clone, Bpaf)]
pub struct ActivationState {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    pub environment: EnvironmentSelect,
}

impl ActivationState {
    pub fn handle(&self, flox: Flox) -> Result<(), anyhow::Error> {
        let concrete_env = self
            .environment
            .detect_concrete_environment(&flox, "Environment path to get activation state")?;

        let activation_state_dir =
            activation_state_dir_path(&flox.runtime_dir, concrete_env.dot_flox_path());
        println!("{}", activation_state_dir.display());
        Ok(())
    }
}
