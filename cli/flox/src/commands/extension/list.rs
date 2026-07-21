use anyhow::Result;
use beta::extensions;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Debug, Bpaf, Clone)]
pub struct List {}

impl List {
    #[instrument(name = "extensions::list", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("extensions::list");

        let extensions = extensions::list(&flox)?;
        if extensions.is_empty() {
            message::plain("No extensions installed.");
            return Ok(());
        }

        println!("{}", super::format::render_header());
        for ext in &extensions {
            let row = super::format::row_from_extension(ext);
            println!("{}", super::format::render_row(&row));
        }
        Ok(())
    }
}
