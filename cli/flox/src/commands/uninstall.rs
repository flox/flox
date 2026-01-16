use anyhow::{Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{Environment, EnvironmentError};
use flox_rust_sdk::providers::migrate::MigrateEnv;
use indoc::formatdoc;
use itertools::Itertools;
use tracing::{debug, info_span, instrument};

use super::services::warn_manifest_changes_for_services;
use super::{EnvironmentSelect, environment_select};
use crate::commands::{EnvironmentSelectError, ensure_floxhub_token, environment_description};
use crate::utils::message;
use crate::utils::tracing::sentry_set_tag;
use crate::{environment_subcommand_metric, subcommand_metric};

// Uninstall installed packages from an environment
#[derive(Bpaf, Clone)]
pub struct Uninstall {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    /// The install IDs or package paths of the packages to remove
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
            ensure_floxhub_token(&mut flox).await?;
        };

        let mut concrete_environment = match self
            .environment
            .detect_concrete_environment(&flox, "Uninstall from")
        {
            Ok(concrete_environment) => concrete_environment,
            Err(EnvironmentSelectError::EnvironmentError(
                ref e @ EnvironmentError::DotFloxNotFound(ref dir),
            )) => {
                let parent = dir.parent().unwrap_or(dir).display();
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init --dir {parent}'"
                })
            },
            Err(e @ EnvironmentSelectError::EnvNotFoundInCurrentDirectory) => {
                bail!(formatdoc! {"
                {e}

                Create an environment with 'flox init' or uninstall packages from an environment found elsewhere with 'flox uninstall {} --dir <path>'",
                self.packages.join(" ")})
            },
            Err(e) => Err(e)?,
        };
        environment_subcommand_metric!("uninstall", concrete_environment);
        concrete_environment.migrate_env(&flox)?;

        let description = environment_description(&concrete_environment)?;

        let span = info_span!(
            "uninstall",
            concrete_environment = %description,
            progress = format!("Uninstalling {} packages", self.packages.len()));

        let attempt =
            span.in_scope(|| concrete_environment.uninstall(self.packages.clone(), &flox))?;

        // Note, you need two spaces between this emoji and the package name
        // otherwise they appear right next to each other.
        self.packages.iter().for_each(|package| {
            message::deleted(format!(
                "'{package}' uninstalled from environment {description}"
            ));
            if let Some(include) = attempt.still_included.get(package) {
                message::info(format!(
                    "'{package}' is still installed by environment '{}'",
                    include.name,
                ));
            }
        });

        warn_manifest_changes_for_services(&flox, &concrete_environment);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::models::environment::path_environment::test_helpers::new_path_environment_from_env_files_in;
    use flox_rust_sdk::models::manifest::typed::Manifest;
    use flox_rust_sdk::providers::catalog::GENERATED_DATA;

    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn uninstall_triggers_migration() {
        let (mut flox, tempdir) = flox_instance();
        flox.features.outputs = true;
        let _env = new_path_environment_from_env_files_in(
            &flox,
            GENERATED_DATA.join("envs/krb5_prereqs"),
            tempdir.path(),
            None,
        );
        Uninstall {
            environment: EnvironmentSelect::Dir(tempdir.path().to_path_buf()),
            packages: vec!["nodejs".to_string()],
        }
        .handle(flox)
        .await
        .unwrap();
        let manifest_path = tempdir.path().join(".flox/env/manifest.toml");
        let manifest_contents = std::fs::read_to_string(manifest_path).unwrap();
        let manifest = Manifest::from_str(&manifest_contents).unwrap();
        assert_eq!(manifest.version, 2.into());
    }
}
