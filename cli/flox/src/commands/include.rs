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
    /// Upgrade environments included in an environment
    #[bpaf(command, header(indoc! {"
        Get the latest contents of included environments and merge them with the
        composing environment.

        The included environments to upgrade can be specified by name,
        or if none are specified, all included environments will be upgraded.
    "}))]
    Upgrade(#[bpaf(external(upgrade))] Upgrade),
}

#[derive(Bpaf, Debug, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Included environments to upgrade
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
            .detect_concrete_environment(&flox, "Upgrade included environments in")?;

        let description = environment_description(&environment)?;

        let span = info_span!(
            "include upgrade",
            progress = format!("Upgrading included environments in environment {description}...")
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
            for upgraded in include_diff {
                message::updated(format!("Upgraded included environment '{upgraded}'"));
            }
        }

        Ok(())
    }
}
