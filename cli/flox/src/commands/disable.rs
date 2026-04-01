use std::path::PathBuf;

use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_core::preference::PreferenceManager;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::find_dot_flox;

use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone, Debug)]
pub struct Disable {
    /// Path to the .flox directory to disable (defaults to current directory)
    #[bpaf(long, argument("PATH"), optional)]
    path: Option<PathBuf>,
}

impl Disable {
    pub fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("disable");

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
        preference_manager.disable(&dot_flox.path)?;

        message::updated(format!(
            "Disabled auto-activation for environment at '{}'",
            dot_flox.path.display()
        ));

        Ok(())
    }
}
