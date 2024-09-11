use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::providers::services::{
    ProcessComposeLogLine,
    ProcessComposeLogStream,
    ProcessComposeLogTail,
    ProcessStates,
    DEFAULT_TAIL,
};
use tracing::instrument;

use crate::commands::services::{guard_service_commands_available, ServicesEnvironment};
use crate::commands::{environment_select, EnvironmentSelect};
use crate::subcommand_metric;

#[derive(Bpaf, Debug, Clone)]
pub struct Logs {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Follow log output
    follow: bool,

    /// Number of lines to show from the end of the logs
    #[bpaf(short('n'), long, argument("num"), fallback(DEFAULT_TAIL))]
    tail: usize,

    /// Which services' logs to view
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Logs {
    #[instrument(name = "logs", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("services::logs");

        let env = ServicesEnvironment::from_environment_selection(&flox, &self.environment)?;
        guard_service_commands_available(&env, &flox.system)?;

        let socket = env.socket();
        let processes = ProcessStates::read(socket)?;

        if self.follow {
            let named_processes = super::processes_by_name_or_default_to_all(
                &processes,
                &env.manifest.services,
                &flox.system,
                &self.names,
            )?;
            let names = named_processes.iter().map(|state| &state.name);
            let log_stream = ProcessComposeLogStream::new(socket, names.clone(), self.tail)?;

            let max_name_length = names.map(|name| name.len()).max().unwrap_or(0);
            for log in log_stream {
                let ProcessComposeLogLine { process, message } = log?;
                println!("{process:<max_name_length$}: {message}",);
            }
        } else {
            let [ref name] = self.names.as_slice() else {
                bail!("A single service name is required when the --follow flag is not specified");
            };

            // Ensure the service exists
            // Avoids attaching to a log of a non-existent service, in which case `process-compose`
            // will block indefinitely.
            if processes.process(name).is_none() {
                return Err(super::service_does_not_exist_error(name))?;
            }

            let tail = ProcessComposeLogTail::new(socket, name, self.tail)?;
            for log in tail {
                let ProcessComposeLogLine { message, .. } = log;
                println!("{message}",);
            }
        }

        Ok(())
    }
}
