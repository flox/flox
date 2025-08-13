use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::display_help;
use crate::config::Config;

mod history;
mod list;
mod rollback;
mod switch;

/// Generations Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum GenerationsCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,

    /// List generations of the environment
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Print the history of the environment
    #[bpaf(command)]
    History(#[bpaf(external(history::history))] history::History),

    /// Switch to the previously active generation
    #[bpaf(command)]
    Rollback(#[bpaf(external(rollback::rollback))] rollback::Rollback),

    /// Switch to the provided generation
    #[bpaf(command)]
    Switch(#[bpaf(external(switch::switch))] switch::Switch),
}

impl GenerationsCommands {
    #[instrument(name = "generations", skip_all)]
    pub fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            GenerationsCommands::Help => {
                display_help(Some("generations".to_string()));
            },
            GenerationsCommands::List(args) => args.handle(flox)?,
            GenerationsCommands::History(args) => args.handle(flox)?,
            GenerationsCommands::Rollback(args) => args.handle(flox)?,
            GenerationsCommands::Switch(args) => args.handle(flox)?,
        }

        Ok(())
    }
}

#[cfg(test)]
pub(super) mod test_helpers {}
