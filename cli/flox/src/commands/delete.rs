use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use indoc::formatdoc;
use miette::{bail, IntoDiagnostic, Result};
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{environment_description, ConcreteEnvironment};
use crate::subcommand_metric;
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::message;

// Delete an environment
#[derive(Bpaf, Clone)]
pub struct Delete {
    /// Delete an environment without confirmation.
    #[bpaf(short, long)]
    force: bool,

    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,
}

impl Delete {
    #[instrument(name = "delete", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("delete");
        let environment = self
            .environment
            .detect_concrete_environment(&flox, "Delete")
            .into_diagnostic()?;

        let description = environment_description(&environment)?;

        if matches!(environment, ConcreteEnvironment::Remote(_)) {
            let message = formatdoc! {"
                Environment {description} was not deleted.

                Remote environments on FloxHub can not yet be deleted.
            "};
            bail!("{message}")
        }

        let confirm = Dialog {
            message: "Are you sure?",
            help_message: Some("Use `-f` to force deletion"),
            typed: Confirm {
                default: Some(false),
            },
        };

        if !self.force && Dialog::can_prompt() && !confirm.prompt().await.into_diagnostic()? {
            bail!("Environment deletion cancelled");
        }

        match environment {
            ConcreteEnvironment::Path(environment) => environment.delete(&flox),
            ConcreteEnvironment::Managed(environment) => environment.delete(&flox),
            ConcreteEnvironment::Remote(_) => unreachable!(),
        }
        .into_diagnostic()?;

        message::deleted(format!("environment {description} deleted"));

        Ok(())
    }
}
