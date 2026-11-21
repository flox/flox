use anyhow::Result;
use bpaf::Bpaf;
use flox_events::{CliEnvironmentPayload, EventKind, EventsHub};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::Environment;
use indoc::indoc;
use tracing::{debug, info_span, instrument};

use super::EnvironmentSelect;
use crate::commands::{display_command_help, environment_description, environment_select};
use crate::environment_subcommand_metric;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message::{self, print_overridden_manifest_fields};

/// Include Commands.
#[derive(Debug, Clone, Bpaf)]
pub enum IncludeCommands {
    /// Prints help information
    #[bpaf(command, hide)]
    Help,
    /// Upgrade an environment with latest changes to its included environments
    #[bpaf(command, long("update"), header(indoc! {"
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
                display_command_help(Some("include".to_string()));
            },
            IncludeCommands::Upgrade(args) => args.handle(flox).await?,
        }

        Ok(())
    }

    pub fn subcommand_name(&self) -> &'static str {
        match self {
            IncludeCommands::Help => "include::help",
            IncludeCommands::Upgrade(_) => "include::upgrade",
        }
    }
}

impl Upgrade {
    #[instrument(name = "upgrade", skip_all)]
    pub async fn handle(self, mut flox: Flox) -> Result<()> {
        flox.resolution_warnings = message::resolution_warning_sink(&flox.system);

        let mut environment = self
            .environment
            .detect_concrete_environment(
                &mut flox,
                "Get latest changes to included environments in",
            )
            .await?;

        environment_subcommand_metric!("include::upgrade", environment);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentIncludeUpgrade(
            CliEnvironmentPayload::new(env_detail_from_concrete(&flox, &environment)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

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
            print_overridden_manifest_fields(&result.new_lockfile);

            // `store_path` is only set when the upgrade wrote a new lockfile.
            if result.store_path.is_some() {
                message::print_default_systems_changed(
                    result.old_lockfile.as_ref(),
                    &result.new_lockfile,
                );
            }

            for name in self.to_upgrade {
                if !include_diff.contains(&name) {
                    message::info(format!("Included environment '{name}' has no changes."));
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::Environment;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_in;
    use flox_rust_sdk::providers::catalog::test_helpers::catalog_replay_client;
    use flox_rust_sdk::utils::logging::test_helpers::test_subscriber_message_only;
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;
    use pretty_assertions::assert_eq;
    use tracing::instrument::WithSubscriber;

    use super::*;
    use crate::commands::EnvironmentSelect;

    /// Upgrading the include brings in a thunderbird that the lockfile doesn't
    /// have, of which the catalog lists no pre-built copy. The build is made
    /// to fail by vars that reference each other, so the warning is all that
    /// is printed before the error.
    #[tokio::test(flavor = "multi_thread")]
    async fn warns_about_package_without_prebuilt_copy_before_building() {
        let (mut flox, tempdir) = flox_instance();
        flox.system = "x86_64-linux".to_string();
        let (subscriber, writer) = test_subscriber_message_only();
        flox.floxhub_client =
            catalog_replay_client(GENERATED_DATA.join("resolve/missing_builds_thunderbird.yaml"))
                .await;
        let dep_path = tempdir.path().join("dep");
        fs::create_dir(&dep_path).unwrap();
        let mut dep = new_path_environment_in(&flox, &with_latest_schema(""), &dep_path);
        dep.lockfile(&flox).unwrap();
        let mut composer = new_path_environment_in(
            &flox,
            &with_latest_schema(indoc! {r#"
                [include]
                environments = [
                  { dir = "dep" },
                ]

                [vars]
                a = "$b"
                b = "$a"
            "#}),
            tempdir.path(),
        );
        composer.lockfile(&flox).unwrap();
        fs::write(
            dep.manifest_path(&flox).unwrap(),
            with_latest_schema(indoc! {r#"
                install.thunderbird.pkg-path = "thunderbird"
            "#}),
        )
        .unwrap();
        dep.lockfile(&flox).unwrap();

        let result = Upgrade {
            environment: EnvironmentSelect::Dir(composer.parent_path().unwrap()),
            to_upgrade: vec!["dep".to_string()],
        }
        .handle(flox)
        .with_subscriber(subscriber)
        .await;

        assert_eq!(
            result.unwrap_err().to_string(),
            "Found a reference cycle in the '[vars]' section: a → b → a.\nRemove one of these references so the variables no longer form a loop."
        );
        assert_eq!(writer.to_string(), indoc! {"
            ! The Flox Catalog lists no pre-built copy of 'thunderbird' for:
              aarch64-darwin, aarch64-linux, x86_64-linux (this system)
            It is built from source there, which can take a long time and may fail.
            A different version may have one: see 'flox show thunderbird'.
        "});
    }
}
