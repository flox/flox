use anyhow::Result;
use bpaf::Bpaf;
use flox_events::{CliEnvironmentPayload, CliPackagePayload, EventKind, EventsHub, Outcome};
use flox_manifest::parsed::latest::{PkgGroups, SelectedOutputs};
use flox_manifest::raw::PackageModification;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::uninstall::UninstallSpec;
use flox_rust_sdk::models::environment::{Environment, EnvironmentError};
use itertools::Itertools;
use tracing::{debug, info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{EnvironmentSelect, environment_select};
use crate::commands::{
    EnvironmentSelectError,
    NoEnvironmentError,
    ensure_auth,
    environment_description,
};
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::utils::tracing::sentry_set_tag;
use crate::{environment_subcommand_metric, subcommand_metric};

// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The install IDs or package paths of the packages to remove.
    /// Supports output specification: "pkg^out,man" to uninstall specific outputs.
    #[bpaf(positional("packages"), some("Must specify at least one package"))]
    packages: Vec<String>,
}

impl Uninstall {
    #[instrument(name = "uninstall", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        // Record subcommand metric prior to environment_subcommand_metric below in case we error
        subcommand_metric!("uninstall");

        sentry_set_tag("packages", self.packages.iter().join(","));

        debug!(
            "uninstalling packages [{}] from {:?}",
            self.packages.as_slice().join(", "),
            self.environment
        );

        // Ensure the user is logged in for the following remote operations
        if let EnvironmentSelect::Remote(_) = self.environment {
            ensure_auth(&mut flox).await?;
        };

        let mut concrete_environment = match self
            .environment
            .detect_concrete_environment(&mut flox, "Uninstall from")
            .await
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::EnvironmentError(EnvironmentError::DotFloxNotFound(
                ref dir,
            ))) => {
                let parent = dir.parent().unwrap_or(dir).display().to_string();
                Err(NoEnvironmentError::Directory(parent))?
            },
            Err(EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => Err(
                NoEnvironmentError::CurrentDirectoryUninstall(self.packages.join(" ")),
            )?,
            Err(e) => Err(e)?,
        };
        environment_subcommand_metric!("uninstall", concrete_environment);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentUninstall(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &concrete_environment)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let description = environment_description(&concrete_environment)?;

        // Parse package arguments into UninstallSpec
        let uninstall_specs: Vec<UninstallSpec> = self
            .packages
            .iter()
            .map(|s| UninstallSpec::parse(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("Failed to parse uninstall spec: {}", e))?;

        let span = info_span!(
            "uninstall",
            concrete_environment = %description,
            progress = format!("Uninstalling {} packages", self.packages.len()));

        let old_lockfile = concrete_environment.existing_lockfile(&flox)?;
        let attempt = span.in_scope(|| concrete_environment.uninstall(uninstall_specs, &flox))?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        for modification in attempt.modifications.iter() {
            let install_id = &modification.install_id;

            match modification.modification {
                PackageModification::Remove => {
                    message::deleted(format!(
                        "'{install_id}' uninstalled from environment {description}"
                    ));
                    if let Some(include) = attempt.still_included.get(install_id) {
                        message::info(format!(
                            "'{install_id}' is still installed by environment '{}'",
                            include.name,
                        ));
                    }
                },
                PackageModification::UpdateOutputs(SelectedOutputs::Specific(ref outputs)) => {
                    message::deleted(format!(
                        "Updated outputs of {install_id} to [{}] from environment {description}",
                        outputs.join(", ")
                    ));
                },
                PackageModification::UpdateOutputs(SelectedOutputs::All(_)) => {
                    unreachable!("if we removed something we shouldn't be left with all outputs")
                },
                // Add is only used for installs, never uninstalls.
                PackageModification::Add(_) => unreachable!(),
            }
        }

        for group in &attempt.removed_pkg_groups {
            message::info(format!(
                "Removed '{}' since it has no packages left",
                PkgGroups::key_path(group),
            ));
        }

        if let Some(new_lockfile) = concrete_environment.existing_lockfile(&flox)? {
            message::print_default_systems_changed(old_lockfile.as_ref(), &new_lockfile);
        }

        warn_manifest_changes_for_services(&flox, &concrete_environment);

        let hub = EventsHub::global();
        for modification in attempt.modifications.iter() {
            // Only a genuine removal is an uninstall. `UpdateOutputs` trims
            // some of a package's outputs without removing the package (the
            // display layer above shows it as "Updated outputs", not
            // "uninstalled"), so it must not emit a `cli.package.uninstall`.
            if !matches!(modification.modification, PackageModification::Remove) {
                continue;
            }
            if let Err(err) = hub.record_event(EventKind::CliPackageUninstall(
                CliPackagePayload::new(modification.install_id.clone(), Outcome::Success),
            )) {
                debug!(error = %err, "Failed to record v2 event");
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment;
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;

    /// Uninstalling locks the environment first, and so resolves thunderbird,
    /// which the user didn't ask to resolve, so the catalog's warning about it
    /// isn't shown. The build is made to fail by vars that reference each
    /// other.
    #[tokio::test(flavor = "multi_thread")]
    async fn does_not_warn_about_package_locked_on_the_way() {
        let (mut flox, _tempdir) = flox_instance();
        flox.system = "x86_64-linux".to_string();
        let (subscriber, writer) = test_subscriber_message_only();
        let environment = new_path_environment(&flox, indoc! {r#"
            version = 1
            install.thunderbird.pkg-path = "thunderbird"

            [vars]
            a = "$b"
            b = "$a"
        "#});
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/missing_builds_thunderbird.yaml"))
                .await;

        let result = Uninstall {
            environment: EnvironmentSelect::Dir(environment.parent_path().unwrap()),
            packages: vec!["thunderbird".to_string()],
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Found a reference cycle in the '[vars]' section: a → b → a.\nRemove one of these references so the variables no longer form a loop."
        );
        assert_eq!(writer.to_string(), "");
    }
}
