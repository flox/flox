use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::ConcreteEnvironment;
use flox_rust_sdk::models::environment::generations::GenerationId;
use indoc::{formatdoc, indoc};
use tracing::{info_span, instrument};

use super::EnvironmentSelect;
use crate::commands::{display_help, environment_description, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::message;

/// Include Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum GenerationsCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,
    /// Upgrade an environment with latest changes to its included environments
    #[bpaf(command, header(indoc! {"

    "}))]
    Rollback(#[bpaf(external(rollback))] Rollback),

    #[bpaf(command, header(indoc! {"

    "}))]
    List(#[bpaf(external(list))] List),
}

impl GenerationsCommands {
    #[instrument(name = "include", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            GenerationsCommands::Help => {
                display_help(Some("include".to_string()));
            },
            GenerationsCommands::Rollback(args) => args.handle(flox).await?,
            GenerationsCommands::List(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

#[derive(Bpaf, Debug, Clone)]
pub struct Rollback {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Generation to roll back to (default: current gen -1)
    #[bpaf(long, argument("generation"))]
    to: Option<GenerationId>,
}

impl Rollback {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        // environment_subcommand_metric!("generations::rollback", self.environment);

        let mut environment = self
            .environment
            .detect_concrete_environment(&flox, "Get latest changes to included environments in")?;

        let env_description = environment_description(&environment)?;

        match environment {
            ConcreteEnvironment::Path(path_environment) => {
                bail!("onlyu supported on managed environments")
            },
            ConcreteEnvironment::Remote(remote_environment) => todo!("im lazy now"),
            ConcreteEnvironment::Managed(mut managed_environment) => {
                let (new_generation, metadata) =
                    managed_environment.switch_generation(&flox, self.to)?;
                let description = metadata.description;
                let created = metadata.created;
                let last_active = metadata
                    .last_active
                    .map(|date| date.to_string())
                    .unwrap_or("never".to_string());
                message::created(formatdoc! {"
                    Rolled back {env_description} to generation {new_generation}:

                    Description: {description}

                    Created: {created}
                    Last Active: {last_active}
                "});
            },
        };

        Ok(())
    }
}

#[derive(Bpaf, Debug, Clone)]
pub struct List {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl List {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        // environment_subcommand_metric!("generations::list", self.environment);

        let mut environment = self
            .environment
            .detect_concrete_environment(&flox, "Get latest changes to included environments in")?;

        let description = environment_description(&environment)?;

        match environment {
            ConcreteEnvironment::Path(path_environment) => {
                bail!("onlyu supported on managed environments")
            },
            ConcreteEnvironment::Remote(remote_environment) => todo!("im lazy now"),
            ConcreteEnvironment::Managed(mut managed_environment) => {
                let metadata = managed_environment.generations_metadata()?;

                for (generation_id, meta) in metadata.generations {
                    let description = meta.description;
                    let created = meta.created;
                    let last_active = meta
                        .last_active
                        .map(|date| date.to_string())
                        .unwrap_or("never".to_string());

                    let current_marker = if Some(generation_id) == metadata.current_gen {
                        " (current)"
                    } else {
                        ""
                    };

                    let message = formatdoc! {"
                        * {generation_id}{current_marker}: {description}
                          Created: {created}
                          Last Active: {last_active}
                    "};

                    print!("{message}")
                }
            },
        };

        Ok(())
    }
}
