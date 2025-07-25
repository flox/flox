use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use list::List;
use tracing::instrument;

use super::display_help;
use crate::config::Config;

mod list;

/// Generations Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum GenerationsCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,

    /// List generations of the selected environment
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] List),
}

impl GenerationsCommands {
    #[instrument(name = "generations", skip_all)]
    pub fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            GenerationsCommands::Help => {
                display_help(Some("generations".to_string()));
            },
            GenerationsCommands::List(args) => args.handle(flox)?,
        }

        Ok(())
    }
}
