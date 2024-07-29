use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{
    ProcessComposeLogLine,
    ProcessComposeLogStream,
    ProcessStates,
};
use tracing::instrument;

use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Bpaf, Debug, Clone)]
pub struct Logs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Which services' logs to view
    #[bpaf(positional("name"))]
    names: Vec<String>,

    /// Follow log output
    follow: bool,
}

impl Logs {
    #[instrument(name = "logs", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("services::logs");

        let env = self
            .environment
            .detect_concrete_environment(&flox, "Services in")?
            .into_dyn_environment();
        let socket = env.services_socket_path(&flox)?;

        let names = if self.names.is_empty() {
            tracing::debug!("no service names provided");
            ProcessStates::read(&socket)?.running_process_names()
        } else {
            self.names
        };

        if !self.follow {
            bail!("printing logs without following is not yet implemented");
        }

        let log_stream = ProcessComposeLogStream::new(socket, &names)?;

        let max_name_length = names.iter().map(|name| name.len()).max().unwrap_or(0);
        for log in log_stream {
            let ProcessComposeLogLine { process, message } = log?;
            println!("{process:<max_name_length$}: {message}",);
        }

        Ok(())
    }
}