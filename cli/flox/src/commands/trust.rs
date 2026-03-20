use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::trust::TrustManager;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::find_dot_flox;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone, Debug)]
pub struct Trust {
    /// Deny trust instead of granting it
    #[bpaf(long)]
    deny: bool,
}

impl Trust {
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("trust");

        let current_dir = std::env::current_dir()?;
        let dot_flox = find_dot_flox(&current_dir)?;

        let Some(dot_flox) = dot_flox else {
            bail!(
                "No '.flox' environment found in the current directory or any parent directory.\n\
                 Use 'flox init' to create one."
            );
        };

        let manager = TrustManager::new(&flox.data_dir);

        if self.deny {
            manager.deny(&dot_flox.path)?;
            message::updated(format!(
                "Denied auto-activation for environment at '{}'",
                dot_flox.path.display()
            ));
        } else {
            manager.trust(&dot_flox.path)?;
            message::updated(format!(
                "Trusted environment at '{}' for auto-activation",
                dot_flox.path.display()
            ));
        }

        Ok(())
    }
}
