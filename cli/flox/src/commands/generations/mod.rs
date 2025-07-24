use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::ConcreteEnvironment;
use flox_rust_sdk::models::environment::generations::{
    AllGenerationsMetadata,
    GenerationsEnvironment,
};
use indoc::formatdoc;
use list::List;
use tracing::instrument;

use super::display_help;
use crate::commands::environment_description;
use crate::config::Config;

mod list;

/// Generations Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum GenerationsCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,

    /// List generations of the selected environment
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] List),
}

impl GenerationsCommands {
    #[instrument(name = "generations", skip_all)]
    pub fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            GenerationsCommands::Help => {
                display_help(Some("generations".to_string()));
            },
            GenerationsCommands::List(args) => args.handle(flox)?,
        }

        Ok(())
    }
}

fn try_get_generations_metadata(env: &ConcreteEnvironment) -> Result<AllGenerationsMetadata> {
    let metadata = match env {
        ConcreteEnvironment::Path(_) => {
            let description = environment_description(env)?;
            bail!(formatdoc! {"
                Generations are only available for environments pushed to floxhub.
                The environment {description} is a local only environment.
            "})
        },
        ConcreteEnvironment::Managed(env) => env.generations_metadata()?,
        ConcreteEnvironment::Remote(env) => env.generations_metadata()?,
    };
    Ok(metadata)
}
