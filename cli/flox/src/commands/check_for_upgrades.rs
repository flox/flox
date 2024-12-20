use std::time::SystemTime;

use anyhow::Result;
use bpaf::{Bpaf, Parser};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::providers::upgrade_checks::{UpgradeInformation, UpgradeInformationGuard};
use serde::de::DeserializeOwned;
use tracing::{debug, info_span, instrument};

use super::UninitializedEnvironment;
use crate::subcommand_metric;

/// By default check once a day
const DEFAULT_TIMEOUT_SECONDS: u64 = 24 * 60 * 60;

#[derive(Bpaf, Clone)]
pub struct CheckForUpgrades {
    /// Skip checking for upgrade if checked less <timeout> seconds ago
    #[bpaf(long, argument("seconds"), fallback(DEFAULT_TIMEOUT_SECONDS))]
    check_timeout: u64,

    #[bpaf(external(parse_uninitialized_environment_json))]
    environment: UninitializedEnvironment,
}

fn parse_uninitialized_environment_json<T: DeserializeOwned>() -> impl Parser<T> {
    bpaf::positional("environment")
        .help("JSON representation of an uninitialized environment to be checked")
        .parse(|string: String| serde_json::from_str::<T>(&string))
}

impl CheckForUpgrades {
    #[instrument(name = "check-upgrade", skip_all)]
    pub async fn handle(self, flox: Flox) -> Result<()> {
        subcommand_metric!("check-upgrade");

        let mut environment = self.environment.into_concrete_environment(&flox)?;

        let upgrade_information =
            UpgradeInformationGuard::for_environment(&flox.cache_dir, environment.dot_flox_path())?;

        // Return if previous information
        // - exists &&
        // - targets the current lockfile &&
        // - has recently been fetched
        // Otherwise, dry-upgrade the environment and store the new information
        if let Some(info) = upgrade_information.info() {
            let environment_lockfile = environment.lockfile(&flox)?;
            if Some(environment_lockfile) == info.result.old_lockfile
                && info.last_checked.elapsed().unwrap().as_secs() < self.check_timeout
            {
                debug!("Recently checked for upgrades. Skipping.");
                return Ok(());
            }
        }

        let Ok(mut locked) = upgrade_information.lock_if_unlocked()? else {
            debug!("Lock already taken. Skipping.");
            return Ok(());
        };

        let result = info_span!("check-upgrade", progress = "Performing dry upgrade")
            .entered()
            .in_scope(|| environment.dry_upgrade(&flox, &[]))?;

        let info = locked.info_mut();

        match info.as_mut() {
            // Check if the resolution didn't change,
            // only update the last checked timestamp
            Some(old_info) if old_info.result.new_lockfile == result.new_lockfile => {
                debug!("Resolution didn't change. Updating last checked timestamp.");
                old_info.last_checked = SystemTime::now()
            },
            Some(_) | None => {
                debug!(diff = ?result.diff(), "Upgrading information with new result.");

                let new_info = UpgradeInformation {
                    last_checked: SystemTime::now(),
                    result,
                };
                let _ = info.insert(new_info);
            },
        };

        locked.commit()?;

        Ok(())
    }
}
