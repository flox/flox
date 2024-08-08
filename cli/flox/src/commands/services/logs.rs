use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{
    ProcessComposeLogLine,
    ProcessComposeLogStream,
    ProcessStates,
};
use tracing::instrument;

use super::supported_environment;
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Bpaf, Debug, Clone)]
pub struct Logs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Follow log output
    follow: bool,

    /// Which services' logs to view
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Logs {
    #[instrument(name = "logs", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("services::logs");

        let env = supported_environment(&flox, &self.environment)?;
        let socket = env.services_socket_path(&flox)?;

        let processes = ProcessStates::read(&socket)?;
        let named_processes = super::processes_by_name_or_default_to_all(&processes, &self.names)?;
        let names = named_processes.iter().map(|state| &state.name);

        if !self.follow {
            bail!("printing logs without following is not yet implemented");
        }

        let log_stream = ProcessComposeLogStream::new(socket, names.clone())?;

        let max_name_length = names.map(|name| name.len()).max().unwrap_or(0);
        for log in log_stream {
            let ProcessComposeLogLine { process, message } = log?;
            println!("{process:<max_name_length$}: {message}",);
        }

        Ok(())
    }
}
