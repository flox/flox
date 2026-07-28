use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

#[derive(Bpaf, Clone, Debug)]
pub struct BetaEnabled {
    #[bpaf(long("option"), argument("arg"))]
    option: Option<String>,
}

impl BetaEnabled {
    #[instrument(name = "beta-enabled", skip_all)]
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        println!("Beta features are enabled.");
        if let Some(option) = self.option {
            println!("'beta-enabled' was called with option '{}'", option);
        }
        Ok(())
    }
}
