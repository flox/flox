use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Stop {
    /// Names of the services to stop
    #[bpaf(positional("name"))]
    names: Vec<String>,
}

impl Stop {
    // TODO: are these nested services->stop?
    #[instrument(name = "stop", skip_all)]
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        // TODO: include spaces?
        subcommand_metric!("services stop");

        message::updated("Stop! In the name of love");

        Ok(())
    }
}
