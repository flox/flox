use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::environment_description;
use crate::subcommand_metric;
use crate::utils::dialog::{Dialog, Spinner};
use crate::utils::message;

// Upgrade packages in an environment
#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// ID of a package or pkg-group name to upgrade
    #[bpaf(positional("package or pkg-group"))]
    groups_or_iids: Vec<String>,
}
impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");
        tracing::debug!(
            to_upgrade = self.groups_or_iids.join(","),
            "upgrading groups and install ids"
        );

        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?;

        let description = environment_description(&concrete_environment)?;

        let mut environment = concrete_environment.into_dyn_environment();

        let result = Dialog {
            message: "Upgrading packages...",
            help_message: None,
            typed: Spinner::new(|| environment.upgrade(&flox, &self.groups_or_iids)),
        }
        .spin()?;

        let upgraded = result.packages;

        if upgraded.is_empty() {
            if self.groups_or_iids.is_empty() {
                message::plain(format!(
                    "ℹ️  No packages need to be upgraded in environment {description}."
                ));
            } else {
                message::plain(format!(
                    "ℹ️  The specified packages do not need to be upgraded in environment {description}."
                 ) );
            }
        } else {
            for package in upgraded {
                message::plain(format!(
                    "⬆️  Upgraded '{package}' in environment {description}."
                ));
            }
        }

        Ok(())
    }
}
