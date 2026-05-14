use anyhow::Result;
use bpaf::Parser;
use flox_rust_sdk::flox::Flox;

#[derive(Bpaf, Clone)]
#[bpaf(hide)]
pub enum BetaCommands {}

pub fn beta_commands() -> impl Parser<BetaCommands> {
    bpaf::fail::<BetaCommands>("no beta subcommands available")
}

impl BetaCommands {
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        match self {}
    }
}
