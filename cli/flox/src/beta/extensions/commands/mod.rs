//! `flox extension` subcommand group.

use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

mod format;
mod install;
mod list;
mod remove;
mod search;
mod upgrade;

/// Manage flox extensions
#[derive(Debug, Bpaf, Clone)]
pub enum ExtensionCommands {
    /// Install an extension
    #[bpaf(command)]
    Install(#[bpaf(external(install::install))] install::Install),

    /// List installed extensions
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Remove an installed extension
    #[bpaf(command)]
    Remove(#[bpaf(external(remove::remove))] remove::Remove),

    /// Search GitHub for flox extensions
    #[bpaf(command)]
    Search(#[bpaf(external(search::search))] search::Search),

    /// Upgrade one or all installed extensions
    #[bpaf(command)]
    Upgrade(#[bpaf(external(upgrade::upgrade))] upgrade::Upgrade),
}

impl ExtensionCommands {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            ExtensionCommands::Install(args) => args.handle(flox).await,
            ExtensionCommands::List(args) => args.handle(flox).await,
            ExtensionCommands::Remove(args) => args.handle(flox).await,
            ExtensionCommands::Search(args) => args.handle(flox).await,
            ExtensionCommands::Upgrade(args) => args.handle(flox).await,
        }
    }
}
