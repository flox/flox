use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::managed_environment::ManagedEnvironment;
use flox_rust_sdk::models::environment::path_environment::PathEnvironment;
use flox_rust_sdk::models::environment::Environment;
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::build::FloxBuildMk;
use flox_rust_sdk::providers::publish::{
    check_build_metadata,
    check_environment_metadata,
    Publisher,
    PublishProvider,
};
use indoc::indoc;
use log::debug;
use tracing::instrument;

use super::{environment_select, EnvironmentSelect};
use crate::commands::{ensure_floxhub_token, ConcreteEnvironment};
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[allow(unused)] // remove when we implement the command
#[derive(Bpaf, Clone)]
pub struct Publish {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(subcommand_or_publish_target))]
    subcommand_or_publish_target: SubcommandOrPublishTarget,
}

#[derive(Debug, Bpaf, Clone)]
enum SubcommandOrPublishTarget {
    PublishTarget {
        /// The package to build.
        /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
        /// If not specified, all packages are built.
        #[bpaf(positional("package"))]
        target: String,
    },
}

impl Publish {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !config.features.unwrap_or_default().publish {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'publish' feature is not enabled.");
        }

        match self.subcommand_or_publish_target {
            SubcommandOrPublishTarget::PublishTarget { target } => {
                let env = self
                    .environment
                    .detect_concrete_environment(&flox, "Publish")?;

                Self::publish(flox, env, target).await
            },
        }
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(mut flox: Flox, env: ConcreteEnvironment, package: String) -> Result<()> {
        subcommand_metric!("publish");

        if !check_package(&env.dyn_environment_ref().lockfile(&flox)?, &package)? {
            bail!("Package '{}' not found in environment", package);
        }

        let env_metadata = match check_environment_metadata(&flox, env.dyn_environment_ref().as_ref()) {
            Ok(env_metadata) => env_metadata,
            Err(e) => bail!("Pre-publish environment checks failed: {}", e.to_string()),
        };

        let build_metadata = match check_build_metadata(&*env.dyn_environment_ref(), &package, &flox.system) {
            Ok(build_metadata) => build_metadata,
            Err(e) => bail!("Pre-publish build checks failed: {}", e.to_string()),
        };

        let publish_provider = PublishProvider::<&FloxBuildMk> {
            build_metadata,
            env_metadata,
            _builder: None,
        };

        ensure_floxhub_token(&mut flox).await?;
        let token = flox.floxhub_token.as_ref().unwrap();

        debug!("publishing package: {}", &package);
        match publish_provider.publish(&flox.catalog_client, token).await {
            Ok(_) => message::created("Package published successfully"),
            Err(e) => bail!("Failed to publish package: {}", e.to_string()),
        }

        Ok(())
    }
}

fn check_package(lockfile: &Lockfile, package: &str) -> Result<bool> {
    let environment_packages = &lockfile.manifest.build;

    if environment_packages.is_empty() {
        bail!(indoc! {"
        No builds found.

        Add a build by modifying the '[build]' section of the manifest with 'flox edit'
        "});
    }

    if !environment_packages.contains_key(package) {
        bail!("Package '{}' not found in environment", package);
    }

    Ok(true)
}
