use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
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
use crate::config::{Config, PublishConfig};
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
    /// URL of store to copy packages to.
    /// Takes precedence over a value from 'flox config'.
    #[bpaf(long, argument("URL"))]
    store_url: Option<Url>,

    /// Path of the key file used to sign packages before copying.
    /// Takes precedence over a value from 'flox config'.
    #[bpaf(long, argument("FILE"))]
    signing_key: Option<PathBuf>,
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
        if !flox.features.publish {
            message::plain("🚧 👷 heja, a new command is in construction here, stay tuned!");
            bail!("'publish' feature is not enabled.");
        }

        let PublishTarget { target } = self.publish_target;
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Publish")?;

        Self::publish(config, flox, env, target, self.cache).await
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(
        config: Config,
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

        let cache = merge_cache_options(config.flox.publish, cache_args)?;
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

/// Merge cache values from config and args, with args taking precedence.
/// Values must be mutually present or absent.
fn merge_cache_options(
    config: Option<PublishConfig>,
    args: Option<CacheArgs>,
) -> Result<Option<NixCopyCache>> {
    let url = args
        .as_ref()
        .and_then(|args| args.store_url.clone())
        .or(config.as_ref().and_then(|cfg| cfg.store_url.clone()));
    let key_file = args
        .as_ref()
        .and_then(|args| args.signing_key.clone())
        .or(config.as_ref().and_then(|cfg| cfg.signing_key.clone()));

    match (url, key_file) {
        (Some(url), Some(key_file)) => Ok(Some(NixCopyCache { url, key_file })),
        (Some(_), None) | (None, Some(_)) => {
            Err(anyhow!("cache URL and key are mutually required options"))
        },
        (None, None) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_cache_options_success() {
        struct TestCase {
            name: &'static str,
            config: Option<PublishConfig>,
            args: Option<CacheArgs>,
            expected: Option<NixCopyCache>,
        }

        let url_args = Url::parse("http://example.com/args").unwrap();
        let url_config = Url::parse("http://example.com/config").unwrap();
        let key_args = PathBuf::from("args.key");
        let key_config = PathBuf::from("config.key");

        let test_cases = vec![
            TestCase {
                name: "None when both None",
                config: None,
                args: None,
                expected: None,
            },
            TestCase {
                name: "args when config None",
                config: None,
                args: Some(CacheArgs {
                    store_url: Some(url_args.clone()),
                    signing_key: Some(key_args.clone()),
                }),
                expected: Some(NixCopyCache {
                    url: url_args.clone(),
                    key_file: key_args.clone(),
                }),
            },
            TestCase {
                name: "config when args None",
                config: Some(PublishConfig {
                    store_url: Some(url_config.clone()),
                    signing_key: Some(key_config.clone()),
                }),
                args: None,
                expected: Some(NixCopyCache {
                    url: url_config.clone(),
                    key_file: key_config.clone(),
                }),
            },
            TestCase {
                name: "args when both Some",
                config: Some(PublishConfig {
                    store_url: Some(url_config.clone()),
                    signing_key: Some(key_config.clone()),
                }),
                args: Some(CacheArgs {
                    store_url: Some(url_args.clone()),
                    signing_key: Some(key_args.clone()),
                }),
                expected: Some(NixCopyCache {
                    url: url_args.clone(),
                    key_file: key_args.clone(),
                }),
            },
            TestCase {
                name: "mix of url from config and key from args",
                config: Some(PublishConfig {
                    store_url: Some(url_config.clone()),
                    signing_key: None,
                }),
                args: Some(CacheArgs {
                    store_url: None,
                    signing_key: Some(key_args.clone()),
                }),
                expected: Some(NixCopyCache {
                    url: url_config.clone(),
                    key_file: key_args.clone(),
                }),
            },
        ];

        for tc in test_cases {
            assert_eq!(
                merge_cache_options(tc.config, tc.args).unwrap(),
                tc.expected,
                "test case: {}",
                tc.name
            );
        }
    }

    #[test]
    fn test_merge_cache_options_error() {
        let url = Url::parse("http://example.com").unwrap();
        let key = PathBuf::from("key");

        let test_cases = vec![
            (
                Some(PublishConfig {
                    store_url: Some(url.clone()),
                    signing_key: None,
                }),
                Some(CacheArgs {
                    store_url: Some(url.clone()),
                    signing_key: None,
                }),
            ),
            (
                Some(PublishConfig {
                    store_url: None,
                    signing_key: Some(key.clone()),
                }),
                Some(CacheArgs {
                    store_url: None,
                    signing_key: Some(key.clone()),
                }),
            ),
        ];

        for (config, args) in test_cases {
            assert_eq!(
                merge_cache_options(config, args).unwrap_err().to_string(),
                "cache URL and key are mutually required options"
            );
        }
    }
}
