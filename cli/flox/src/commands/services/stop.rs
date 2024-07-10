use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::stop_services;
use tracing::instrument;

use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Stop {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to stop
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Stop {
    // TODO: are these nested services->stop?
    #[instrument(name = "stop", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        // TODO: include spaces?
        subcommand_metric!("services stop");

        let env = self
            .environment
            .detect_concrete_environment(&flox, "Services in")?
            .into_dyn_environment();
        let socket = env.services_socket_path(&flox)?;

        stop_services(socket, self.names)?;

        message::updated("Stop! In the name of love");

        Ok(())
    }
}
