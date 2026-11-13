//! `flox extension` subcommand group.

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

use crate::commands::display_command_help;

mod format;
mod install;
mod list;
mod remove;

/// Manage flox extensions
#[derive(Debug, Bpaf, Clone)]
pub enum ExtensionCommands {
    /// Print help information
    #[bpaf(command, hide)]
    Help,

    /// Install an extension
    #[bpaf(command)]
    Install(#[bpaf(external(install::install))] install::Install),

    /// List installed extensions
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Remove an installed extension
    #[bpaf(command)]
    Remove(#[bpaf(external(remove::remove))] remove::Remove),
}

impl ExtensionCommands {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            ExtensionCommands::Help => {
                display_command_help(Some("extension".to_string()));
                Ok(())
            },
            ExtensionCommands::Install(args) => args.handle(flox).await,
            ExtensionCommands::List(args) => args.handle(flox).await,
            ExtensionCommands::Remove(args) => args.handle(flox).await,
        }
    }
}
