use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use tracing::{info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{environment_select, EnvironmentSelect};
use crate::commands::{ensure_floxhub_token, environment_description};
use crate::subcommand_metric;
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
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        subcommand_metric!("upgrade");
        tracing::debug!(
            to_upgrade = self.groups_or_iids.join(","),
            "upgrading groups and install ids"
        );

        // Ensure the user is logged in for the following remote operations
        if let EnvironmentSelect::Remote(_) = self.environment {
            ensure_floxhub_token(&mut flox).await?;
        };

        let concrete_environment = self
            .environment
            .detect_concrete_environment(&flox, "Upgrade")?;

        let description = environment_description(&concrete_environment)?;

        let mut environment = concrete_environment.into_dyn_environment();

        let span = info_span!(
            "upgrade",
            environment = %description,
            progress = format!(
                "Upgrading {} package(s) or group(s)",
                if self.groups_or_iids.is_empty() {
                    "all".to_string()
                } else {
                    format!("{}", self.groups_or_iids.len())
                }
            )
        );
        let result = span.in_scope(|| {
            environment.upgrade(
                &flox,
                &self
                    .groups_or_iids
                    .iter()
                    .map(String::as_str)
                    .collect::<Vec<_>>(),
            )
        })?;

        let upgraded = result.packages().collect::<Vec<_>>();

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

            warn_manifest_changes_for_services(&flox, environment.as_ref());
        }

        Ok(())
    }
}
