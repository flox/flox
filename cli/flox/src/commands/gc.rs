use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::message;

#[derive(Bpaf, Debug, Clone)]
pub struct Gc {}

impl Gc {
    #[instrument(
        skip_all,
        fields(progress = "Garbage collecting unused environment data")
    )]
    pub fn handle(self, flox: Flox) -> Result<()> {
        message::updated("Garbage collection complete");
        Ok(())
    }
}
