use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use indoc::indoc;
use tracing::{info_span, instrument};

use super::EnvironmentSelect;
use crate::commands::{display_help, environment_description, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::message;

/// Include Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum IncludeCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,
    /// Upgrade an environment with latest changes to its included environments
    #[bpaf(command, header(indoc! {"
        Get the latest contents of included environments and merge them with the
        composing environment.



        If the names of specific included environments are provided,
        only changes for those environments will be fetched.
        If no names are provided, changes will be fetched for all included
        environments.
    "}))]
    Upgrade(#[bpaf(external(upgrade))] Upgrade),
}

#[derive(Bpaf, Debug, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Name of included environment to check for changes
    #[bpaf(positional("included environment"))]
    to_upgrade: Vec<String>,
}

impl IncludeCommands {
    #[instrument(name = "include", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        match self {
            IncludeCommands::Help => {
                display_help(Some("include".to_string()));
            },
            IncludeCommands::Upgrade(args) => args.handle(flox).await?,
        }

        Ok(())
    }
}

impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        environment_subcommand_metric!("include::upgrade", self.environment);

        let mut environment = self
            .environment
            .detect_concrete_environment(&flox, "Get latest changes to included environments in")?;

        let description = environment_description(&environment)?;

        let span = info_span!(
            "include upgrade",
            progress = format!(
                "Getting latest changes to environments included in environment {description}..."
            )
        );
        let result =
            span.in_scope(|| environment.include_upgrade(&flox, self.to_upgrade.clone()))?;

        let include_diff = result.include_diff();
        if include_diff.is_empty() {
            if self.to_upgrade.is_empty() {
                message::info("No included environments have changes.");
            } else {
                for name in self.to_upgrade {
                    message::info(format!("Included environment '{name}' has no changes."))
                }
            }
        } else {
            let mut message = format!("Upgraded {description} with latest changes to:");
            for upgraded in &include_diff {
                message.push_str(&format!("\n- '{upgraded}'"));
            }
            message::updated(message);
            for name in self.to_upgrade {
                if !include_diff.contains(&name) {
                    message::info(format!("Included environment '{name}' has no changes."));
                }
            }
        }

        Ok(())
    }
}
