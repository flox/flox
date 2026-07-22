use anyhow::Result;
use beta::extensions;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Debug, Bpaf, Clone)]
pub struct Remove {
    /// Name of the installed extension to remove
    #[bpaf(positional("NAME"))]
    name: String,
}

impl Remove {
    #[instrument(name = "extensions::remove", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::remove");

        extensions::remove(&flox, &self.name)?;
        message::updated(format!("Removed flox-{}", self.name));
        Ok(())
    }
}
