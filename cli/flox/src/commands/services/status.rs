use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::ProcessStates;
use tracing::instrument;

use super::supported_environment;
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Bpaf, Debug, Clone)]
pub struct Status {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Names of the services to query
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Status {
    #[instrument(name = "status", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("services::status");

        let env = supported_environment(&flox, self.environment)?;
        let socket = env.services_socket_path(&flox)?;

        let mut states = ProcessStates::read(socket)?;
        if !self.names.is_empty() {
            states.filter_names(self.names);
        }

        println!("{}", states);

        Ok(())
    }
}
