use std::path::PathBuf;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::preference::PreferenceManager;
use flox_core::trust::TrustManager;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{EnvironmentPointer, find_dot_flox};

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone, Debug)]
pub struct Allow {
    /// Path to the .flox directory to allow (defaults to current directory)
    #[bpaf(long, argument("PATH"), optional)]
    path: Option<PathBuf>,
}

impl Allow {
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("allow");

        let search_dir = match &self.path {
            Some(p) => p.clone(),
            None => std::env::current_dir()?,
        };
        let dot_flox = find_dot_flox(&search_dir)?;

        let Some(dot_flox) = dot_flox else {
            bail!(
                "No '.flox' environment found at '{}' or any parent directory.\n\
                 Use 'flox init' to create one.",
                search_dir.display()
            );
        };

        let preference_manager = PreferenceManager::new(&flox.state_dir);
        preference_manager.allow(&dot_flox.path)?;

        // For local environments, trust is implicit when allowing
        if matches!(dot_flox.pointer, EnvironmentPointer::Path(_)) {
            let trust_manager = TrustManager::new(&flox.data_dir);
            trust_manager.trust(&dot_flox.path)?;
        }

        message::updated(format!(
            "Allowed auto-activation for environment at '{}'",
            dot_flox.path.display()
        ));

        Ok(())
    }
}
