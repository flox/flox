use std::path::PathBuf;

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
use crate::subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Publish {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(cache_args), optional)]
    cache: Option<CacheArgs>,

    #[bpaf(external(publish_target))]
    publish_target: PublishTarget,
}

#[derive(Debug, Bpaf, Clone)]
struct CacheArgs {
    #[bpaf(long("cache"))]
    url: Url,

    #[bpaf(long("signing-key"))]
    key_file: PathBuf,
}

#[derive(Debug, Bpaf, Clone)]
struct PublishTarget {
    /// The package to publish.
    /// Corresponds to entries in the 'build' table in the environment's manifest.toml.
    #[bpaf(positional("package"))]
    target: String,
}

impl Publish {
    pub async fn handle(self, flox: Flox) -> Result<()> {
        if !flox.features.publish {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'publish' feature is not enabled.");
        }

        let PublishTarget { target } = self.publish_target;
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Publish")?;

        Self::publish(flox, env, target, self.cache).await
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(
        mut flox: Flox,
        mut env: ConcreteEnvironment,
        package: String,
        cache_args: Option<CacheArgs>,
    ) -> Result<()> {
        subcommand_metric!("publish");

        if !check_package(&env.lockfile(&flox)?, &package)? {
            bail!("Package '{}' not found in environment", package);
        }

        if matches!(env, ConcreteEnvironment::Remote(_)) {
            bail!("Unsupported environment type");
        }

        let env_metadata = check_environment_metadata(&flox, &mut env)?;

        let build_metadata = check_build_metadata(&env, &package)?;

        let cache = cache_args.map(|args| NixCopyCache {
            url: args.url,
            key_file: args.key_file,
        });

        let publish_provider = PublishProvider::<&FloxBuildMk, &NixCopyCache> {
            build_metadata,
            env_metadata,
            cache: cache.as_ref(),
            _builder: None,
        };

        let token = ensure_floxhub_token(&mut flox).await?;

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
