use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;

// Update the global base catalog or an environment's base catalog
#[derive(Bpaf, Clone)]
pub struct Update;

impl Update {
    pub async fn handle(self, _flox: Flox) -> Result<()> {
        bail!(
            "'flox update' has been removed.\n\nTo upgrade packages, run 'flox upgrade'. See flox-upgrade(1) for more."
        );
    }
}
