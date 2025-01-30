use anyhow::Result;
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::SingleSystemUpgradeDiff;
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{environment_select, EnvironmentSelect};
use crate::commands::{ensure_floxhub_token, environment_description};
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

// Upgrade packages in an environment
#[derive(Bpaf, Clone)]
pub struct Upgrade {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// Show available upgrades but do not apply them
    #[bpaf(long)]
    dry_run: bool,

    /// ID of a package or pkg-group name to upgrade
    #[bpaf(positional("package or pkg-group"))]
    groups_or_iids: Vec<String>,
}
impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        environment_subcommand_metric!("upgrade", self.environment);
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

        let progress_message = {
            let num_upgrades = if self.groups_or_iids.is_empty() {
                "all".to_string()
            } else {
                format!("{}", self.groups_or_iids.len())
            };

            let dry_prefix = if self.dry_run { "Dry run: " } else { "" };

            format!("{dry_prefix}Upgrading {num_upgrades} package(s) or group(s)")
        };

        let span = info_span!(
            "upgrade",
            environment = %description,
            progress = %progress_message
        );
        let result = span.in_scope(|| {
            let groups_or_iids = &self
                .groups_or_iids
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>();

            if self.dry_run {
                environment.dry_upgrade(&flox, groups_or_iids)
            } else {
                environment.upgrade(&flox, groups_or_iids)
            }
        })?;

        let diff = result.diff();

        if diff.is_empty() {
            if self.groups_or_iids.is_empty() {
                message::plain(format!(
                    "No upgrades available for packages in {description}."
                ));
            } else {
                message::plain(format!(
                    "No upgrades available for the specified packages in {description}."
                ));
            }
            return Ok(());
        }

        let diff_for_system = result.diff_for_system(&flox.system);

        let rendered_diff = render_diff(&diff_for_system);
        let num_changes_for_system = diff_for_system.len();

        if self.dry_run {
            if diff_for_system.is_empty() {
                message::plain(formatdoc! {"
                    Upgrades are not available for {description} on this system, but upgrades are
                    available for other systems supported by this environment."});
                if self.groups_or_iids.is_empty() {
                } else {
                    message::plain(format!(
                        "No upgrades available for the specified packages in {description}."
                    ));
                }
                return Ok(());
            }
            message::plain(formatdoc! {"
                Dry run: Upgrades available for {num_changes_for_system} package(s) in {description}:
                {rendered_diff}

                To apply these changes, run upgrade without the '--dry-run' flag.
            "});

            return Ok(());
        }

        if diff_for_system.is_empty() {
            message::plain(formatdoc! {"
            ✅  Upgraded {description}.
            Upgrades were not available for this system, but upgrades were applied for other
            systems supported by this environment."});
        } else {
            message::plain(formatdoc! {"
            ✅  Upgraded {num_changes_for_system} package(s) in {description}:
            {rendered_diff}
            "});
        }

        warn_manifest_changes_for_services(&flox, environment.as_ref());

        Ok(())
    }
}

/// Render a diff of locked packages before and after an upgrade
fn render_diff(diff: &SingleSystemUpgradeDiff) -> String {
    diff.iter()
        .map(|(_, (before, after))| {
            let install_id = before.install_id();
            let old_version = before.version().unwrap_or("unknown");
            let new_version = after.version().unwrap_or("unknown");

            if new_version == old_version {
                format!("- {install_id}: {old_version}")
            } else {
                format!("- {install_id}: {old_version} -> {new_version}")
            }
        })
        .join("\n")
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::{
        new_path_environment,
        new_path_environment_from_env_files,
    };
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::manifest::raw::PackageToInstall;
    use flox_rust_sdk::providers::catalog::test_helpers::reset_mocks_from_file;
    use flox_rust_sdk::providers::catalog::GENERATED_DATA;
    use flox_rust_sdk::utils::logging::test_helpers::CollectingWriter;
    use indoc::indoc;
    use tracing::instrument::WithSubscriber;
    use tracing::Subscriber;
    use tracing_subscriber::filter::FilterFn;
    use tracing_subscriber::layer::SubscriberExt;

    use super::*;
    use crate::commands::EnvironmentSelect;

    fn test_subscriber() -> (impl Subscriber, CollectingWriter) {
        let (subscriber, writer) = flox_rust_sdk::utils::logging::test_helpers::test_subscriber();
        let subscriber = subscriber.with(FilterFn::new(|metadata| {
            metadata.target() == "flox::utils::message"
        }));
        (subscriber, writer)
    }

    /// Check message printed when there are no upgrades available
    #[tokio::test]
    async fn confirmation_when_up_to_date() {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let environment =
            new_path_environment_from_env_files(&flox, GENERATED_DATA.join("envs/hello"));

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        Upgrade {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            dry_run: true,
            groups_or_iids: Vec::new(),
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await
        .unwrap();

        let printed = writer.to_string();

        assert_eq!(printed, "No upgrades available for packages in 'name'.\n");
    }

    /// Run an upgrade of an environment that only has upgrades on other systems
    async fn run_upgrade_with_upgrades_on_other_system(dry_run: bool) -> String {
        let (mut flox, _tempdir) = flox_instance();
        let (subscriber, writer) = test_subscriber();

        let mut environment = new_path_environment(&flox, "version = 1");

        #[cfg(target_os = "macos")]
        reset_mocks_from_file(&mut flox.catalog_client, "resolve/old_linux_hello.json");
        #[cfg(target_os = "linux")]
        reset_mocks_from_file(&mut flox.catalog_client, "resolve/old_darwin_hello.json");

        environment
            .install(
                &[PackageToInstall::parse(&flox.system, "hello").unwrap()],
                &flox,
            )
            .unwrap();

        reset_mocks_from_file(&mut flox.catalog_client, "resolve/hello.json");
        Upgrade {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            dry_run,
            groups_or_iids: Vec::new(),
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await
        .unwrap();

        writer.to_string()
    }

    #[tokio::test]
    async fn upgrade_on_other_system() {
        assert_eq!(
            run_upgrade_with_upgrades_on_other_system(false).await,
            indoc! {"
            ✅  Upgraded 'name'.
            Upgrades were not available for this system, but upgrades were applied for other
            systems supported by this environment.
            "}
        );
    }

    #[tokio::test]
    async fn upgrade_dry_run_on_other_system() {
        assert_eq!(
            run_upgrade_with_upgrades_on_other_system(true).await,
            indoc! {"
            Upgrades are not available for 'name' on this system, but upgrades are
            available for other systems supported by this environment.
            "}
        );
    }
}
