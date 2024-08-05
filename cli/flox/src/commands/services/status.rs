use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{ProcessStates, ProcessStatesDisplay};
use tracing::instrument;

use super::supported_environment;
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Bpaf, Debug, Clone)]
pub struct Status {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Display output as JSON
    #[bpaf(long)]
    json: bool,

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

        let procs: ProcessStatesDisplay = if self.names.is_empty() {
            ProcessStates::read(socket)?.into()
        } else {
            ProcessStates::read_names(socket, self.names)?.into()
        };

        if self.json {
            for proc in procs {
                let line = serde_json::to_string(&proc)?;
                println!("{line}");
            }
        } else {
            println!("{procs}");
        }

        Ok(())
    }
}
