use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::display_help;
use crate::config::Config;

mod list;
mod rollback;
mod switch;

/// Generations Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum GenerationsCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,

    /// List generations of the selected environment
    #[bpaf(command)]
    List(#[bpaf(external(list::list))] list::List),

    /// Switch to the last active generation
    #[bpaf(command)]
    Rollback(#[bpaf(external(rollback::rollback))] rollback::Rollback),

    /// Switch to the provided generation
    #[bpaf(command)]
    Switch(#[bpaf(external(switch::switch))] switch::Switch),
}

impl GenerationsCommands {
    #[instrument(name = "generations", skip_all)]
    pub fn handle(self, _config: Config, flox: Flox) -> Result<()> {
        match self {
            GenerationsCommands::Help => {
                display_help(Some("generations".to_string()));
            },
            GenerationsCommands::List(args) => args.handle(flox)?,
            GenerationsCommands::Rollback(args) => args.handle(flox)?,
            GenerationsCommands::Switch(args) => args.handle(flox)?,
        }

        Ok(())
    }
}

#[cfg(test)]
pub(super) mod test_helpers {
    use chrono::DateTime;
    use flox_rust_sdk::models::environment::generations::{
        AllGenerationsMetadata,
        GenerationId,
        SingleGenerationMetadata,
    };

    pub(super) fn mock_generations(active: GenerationId) -> AllGenerationsMetadata {
        let active_ts = Some(DateTime::default() + chrono::Duration::hours(4));

        AllGenerationsMetadata::new(active, [
            (1.into(), SingleGenerationMetadata {
                created: DateTime::default() + chrono::Duration::hours(1),
                last_active: if active == 1.into() { active_ts } else { None },
                description: "Generation 1 description".to_string(),
            }),
            (2.into(), SingleGenerationMetadata {
                created: DateTime::default() + chrono::Duration::hours(2),
                last_active: if active == 2.into() {
                    active_ts
                } else {
                    Some(DateTime::default() + chrono::Duration::hours(2))
                },
                description: "Generation 2 description".to_string(),
            }),
            (3.into(), SingleGenerationMetadata {
                created: DateTime::default() + chrono::Duration::hours(3),
                last_active: if active == 3.into() {
                    active_ts
                } else {
                    Some(DateTime::default() + chrono::Duration::hours(3))
                },
                description: "Generation 3 description".to_string(),
            }),
        ])
    }
}
