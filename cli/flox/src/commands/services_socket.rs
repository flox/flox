use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;

use super::{EnvironmentSelect, environment_select};

#[derive(Debug, Clone, Bpaf)]
pub struct ServicesSocket {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    pub environment: EnvironmentSelect,
}

impl ServicesSocket {
    pub fn handle(&self, flox: Flox) -> Result<(), anyhow::Error> {
        let concrete_env = self
            .environment
            .detect_concrete_environment(&flox, "Environment path to get services socket")?;

        let socket_path = concrete_env.services_socket_path(&flox)?;
        println!("{}", socket_path.display());
        Ok(())
    }
}
