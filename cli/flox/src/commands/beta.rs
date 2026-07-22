use anyhow::Result;
use beta::beta_enabled;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

use crate::commands::extension;

#[derive(Bpaf, Clone)]
#[bpaf(hide)]
pub enum BetaCommands {
    #[bpaf(command("beta-enabled"), hide)]
    BetaEnabled(#[bpaf(external(beta_enabled::beta_enabled))] beta_enabled::BetaEnabled),

    /// Manage flox extensions
    #[bpaf(command("extension"), hide)]
    Extension(#[bpaf(external(extension::extension_commands))] extension::ExtensionCommands),
}

impl BetaCommands {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            BetaCommands::BetaEnabled(args) => args.handle(flox).await,
            BetaCommands::Extension(args) => args.handle(flox).await,
        }
    }

    pub fn subcommand_name(&self) -> &'static str {
        match self {
            BetaCommands::BetaEnabled(_) => "beta-enabled",
            BetaCommands::Extension(_) => "extension",
        }
    }
}
