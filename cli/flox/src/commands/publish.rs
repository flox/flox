use anyhow::{bail, Result};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::providers::build::FloxBuildMk;
use flox_rust_sdk::providers::publish::{
    check_build_metadata,
    check_environment_metadata,
    NixCopyCache,
    PublishProvider,
    Publisher,
};
use indoc::{formatdoc, indoc};
use log::debug;
use tracing::instrument;
use url::Url;

use super::{environment_select, EnvironmentSelect};
use crate::commands::ensure_floxhub_token;
use crate::config::Config;
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Publish {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(long)]
    cache: Option<Url>,

    #[bpaf(long)]
    key_file: Option<String>,

    #[bpaf(external(publish_target))]
    publish_target: PublishTarget,
}

#[derive(Debug, Bpaf, Clone)]
struct PublishTarget {
    /// The package to publish.
    /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
    #[bpaf(positional("package"))]
    target: String,
}

impl Publish {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        if !config.features.unwrap_or_default().publish {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'publish' feature is not enabled.");
        }

        let PublishTarget { target } = self.publish_target;
        if self.cache.is_some() && self.key_file.is_none() {
            bail!("--cache requires a signing key to be provided via --key-file.");
        }
        {
            let env = self
                .environment
                .detect_concrete_environment(&flox, "Publish")?;

            Self::publish(flox, env, target, self.cache, self.key_file).await
        }
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(
        mut flox: Flox,
        mut env: ConcreteEnvironment,
        package: String,
        cache_uri: Option<Url>,
        key_file: Option<String>,
    ) -> Result<()> {
        subcommand_metric!("publish");

        if !check_package(&env.lockfile(&flox)?, &package)? {
            bail!("Package '{}' not found in environment", package);
        }

        if matches!(env, ConcreteEnvironment::Remote(_)) {
            bail!("Unsupported environment type");
        }

        let env_metadata =
            check_environment_metadata(&flox, &mut env).or_else(|e| bail!(e.to_string()))?;

        let build_metadata =
            check_build_metadata(&env, &package, &flox.system).or_else(|e| bail!(e.to_string()))?;

        let cache = match (cache_uri, key_file) {
            (Some(uri), Some(key_file)) => Some(NixCopyCache { uri, key_file }),
            _ => None,
        };

        let publish_provider = PublishProvider::<&FloxBuildMk, &NixCopyCache> {
            build_metadata,
            env_metadata,
            cache: cache.as_ref(),
            _builder: None,
        };

        ensure_floxhub_token(&mut flox).await?;
        let token = flox
            .floxhub_token
            .as_ref()
            .expect("should be authenticated to FloxHub");
        let catalog_name = token.handle().to_string();

        debug!("publishing package: {}", &package);
        match publish_provider
            .publish(&flox.catalog_client, &catalog_name)
            .await
        {
            Ok(_) => message::updated(formatdoc! {"
                Package published successfully.

                Use 'flox install {catalog_name}/{package}' to install it.
                "}),
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
