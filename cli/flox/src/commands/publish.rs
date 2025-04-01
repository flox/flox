use std::path::PathBuf;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::lockfile::Lockfile;
use flox_rust_sdk::models::manifest::typed::{Inner, Manifest};
use flox_rust_sdk::providers::build::FloxBuildMk;
use flox_rust_sdk::providers::catalog::ClientTrait;
use flox_rust_sdk::providers::publish::{
    NixCopyCache,
    PublishProvider,
    Publisher,
    check_build_metadata,
    check_environment_metadata,
};
use indoc::{formatdoc, indoc};
use tracing::{debug, instrument};
use url::Url;

use super::{EnvironmentSelect, environment_select};
use crate::commands::ensure_floxhub_token;
use crate::config::{Config, PublishConfig};
use crate::environment_subcommand_metric;
use crate::utils::message;

#[derive(Bpaf, Clone)]
pub struct Publish {
    #[bpaf(external(environment_select), fallback(Default::default()))]
    environment: EnvironmentSelect,

    #[bpaf(external(cache_args))]
    cache: CacheArgs,

    /// Only publish the metadata of the package, and do not upload the artifact
    /// itself.
    ///
    /// With this option present, a signing key is not required.
    #[bpaf(long)]
    metadata_only: bool,

    #[bpaf(external(publish_target), optional)]
    publish_target: Option<PublishTarget>,
}

#[derive(Debug, Bpaf, Clone, Default)]
struct CacheArgs {
    /// URL of store to copy packages to.
    /// Takes precedence over a value from 'flox config'.
    #[bpaf(long, argument("URL"), hide)]
    store_url: Option<Url>,

    /// Which catalog to publish to.
    /// Takes precedence over the default value of the user's GitHub handle.
    #[bpaf(short, long, argument("NAME"))]
    catalog: Option<String>,

    /// Path of the key file used to sign packages before copying.
    /// Takes precedence over a value from 'flox config'.
    #[bpaf(long, argument("FILE"))]
    signing_private_key: Option<PathBuf>,
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

        environment_subcommand_metric!("publish", self.environment);
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Publish")?;
        let target = Self::get_publish_target(
            &env.manifest(&flox)
                .context("failed to get environment manifest")?,
            &self.publish_target,
        )?;
        Self::publish(config, flox, env, target, self.metadata_only, self.cache).await
    }

    fn get_publish_target(
        manifest: &Manifest,
        target_arg: &Option<PublishTarget>,
    ) -> Result<String> {
        let target = if target_arg.is_none() {
            match manifest.build.inner().len() {
                0 => {
                    bail!("Cannot publish without a build specified");
                },
                1 => manifest
                    .build
                    .inner()
                    .keys()
                    .next()
                    .expect("expect there to be at least one build")
                    .clone(),
                _ => {
                    bail!("Must specify an artifact to publish");
                },
            }
        } else {
            target_arg
                .as_ref()
                .expect("already checked that publish target existed")
                .target
                .clone()
        };
        Ok(target)
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(
        config: Config,
        mut flox: Flox,
        mut env: ConcreteEnvironment,
        package: String,
        metadata_only: bool,
        cache_args: CacheArgs,
    ) -> Result<()> {
        let lockfile = env.lockfile(&flox)?.into();
        if !check_target_exists(&lockfile, &package)? {
            bail!("Package '{}' not found in environment", package);
        }

        // Fail as early as possible if the user isn't authenticated or doesn't
        // belong to an org with a catalog.
        let token = ensure_floxhub_token(&mut flox).await?;
        let catalog_name = cache_args
            .catalog
            .clone()
            .unwrap_or(token.handle().to_string());

        let path_env = match env {
            ConcreteEnvironment::Path(path_env) => path_env,
            _ => bail!("Unsupported environment type"),
        };

        // Check the environment for appropriate state to build and publish
        let env_metadata = check_environment_metadata(&flox, &path_env, &package)?;

        let build_metadata =
            check_build_metadata(&flox, &env_metadata, &path_env, &FloxBuildMk, &package)?;

        let cache = if metadata_only {
            None
        } else {
            merge_cache_options(&flox, &catalog_name, config.flox.publish, cache_args).await?
        };

        let publish_provider = PublishProvider::<&NixCopyCache> {
            env_metadata,
            build_metadata,
            cache: cache.as_ref(),
        };

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

fn check_target_exists(lockfile: &Lockfile, package: &str) -> Result<bool> {
    let environment_packages = &lockfile.manifest.build;

    if environment_packages.inner().is_empty() {
        bail!(indoc! {"
        No builds found.

        Add a build by modifying the '[build]' section of the manifest with 'flox edit'
        "});
    }

    if !environment_packages.inner().contains_key(package) {
        bail!("Package '{}' not found in environment", package);
    }

    Ok(true)
}

/// Merge cache values from config and args, with args taking precedence.
/// If there's an ingress URI, there must be a signing key.
async fn merge_cache_options(
    flox: &Flox,
    catalog_name: impl AsRef<str> + Send + Sync,
    config: Option<PublishConfig>,
    args: CacheArgs,
) -> Result<Option<NixCopyCache>> {
    let url = match args.store_url {
        Some(url) => Some(url),
        // Get the ingress URI for this catalog if it has one configured.
        None => flox.catalog_client.get_ingress_uri(catalog_name).await?,
    };
    let key_file = args.signing_private_key.or(config
        .as_ref()
        .and_then(|cfg| cfg.signing_private_key.clone()));

    match (url, key_file) {
        (Some(url), Some(key_file)) => Ok(Some(NixCopyCache { url, key_file })),
        (Some(_), None) => {
            let msg = formatdoc! { "
               A signing key is required to upload artifacts.

               You can supply a signing key by either:
               - Providing a path to a key with the '--signing-private-key' option.
               - Setting it in the config via 'flox config --set publish.signing-key <path>'

               Or you can publish without uploading artifacts via the '--metadata-only' option.
            "};
            Err(anyhow!(msg))?
        },
        // We don't care if you have a signing key when there's no ingress URI
        (None, _) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use flox_rust_sdk::flox::test_helpers::flox_instance;
    use flox_rust_sdk::providers::catalog::test_helpers::reset_mocks;
    use flox_rust_sdk::providers::catalog::{PublishResponse, Response};

    use super::*;

    #[tokio::test]
    async fn test_merge_cache_options_success() {
        struct TestCase {
            name: &'static str,
            config: Option<PublishConfig>,
            args: CacheArgs,
            ingress_uri: Option<String>,
            expected: Result<Option<NixCopyCache>>,
        }

        let url_args = Url::parse("http://example.com/args").unwrap();
        let url_response_str = "http://example.com/response";
        let url_response = Url::parse(url_response_str).unwrap();
        let key_args = PathBuf::from("args.key");
        let key_config = PathBuf::from("config.key");

        let test_cases = vec![
            TestCase {
                name: "None when all None",
                config: None,
                args: CacheArgs {
                    store_url: None,
                    catalog: None,
                    signing_private_key: None,
                },
                ingress_uri: None,
                expected: Ok(None),
            },
            TestCase {
                name: "args when ingress_uri None",
                config: None,
                args: CacheArgs {
                    store_url: Some(url_args.clone()),
                    catalog: None,
                    signing_private_key: Some(key_args.clone()),
                },
                ingress_uri: None,
                expected: Ok(Some(NixCopyCache {
                    url: url_args.clone(),
                    key_file: key_args.clone(),
                })),
            },
            TestCase {
                name: "ingress_uri when args None",
                config: Some(PublishConfig {
                    signing_private_key: Some(key_config.clone()),
                }),
                args: CacheArgs {
                    store_url: None,
                    catalog: None,
                    signing_private_key: None,
                },
                ingress_uri: Some(url_response_str.to_string()),
                expected: Ok(Some(NixCopyCache {
                    url: url_response.clone(),
                    key_file: key_config.clone(),
                })),
            },
            TestCase {
                name: "args when both Some",
                config: Some(PublishConfig {
                    signing_private_key: Some(key_config.clone()),
                }),
                args: CacheArgs {
                    store_url: Some(url_args.clone()),
                    catalog: None,
                    signing_private_key: Some(key_args.clone()),
                },
                ingress_uri: Some(url_response_str.to_string()),
                expected: Ok(Some(NixCopyCache {
                    url: url_args.clone(),
                    key_file: key_args.clone(),
                })),
            },
            TestCase {
                name: "mix of url from response and key from args",
                config: Some(PublishConfig {
                    signing_private_key: None,
                }),
                args: CacheArgs {
                    store_url: None,
                    catalog: None,
                    signing_private_key: Some(key_args.clone()),
                },
                ingress_uri: Some(url_response_str.to_string()),
                expected: Ok(Some(NixCopyCache {
                    url: url_response.clone(),
                    key_file: key_args.clone(),
                })),
            },
            TestCase {
                name: "no error when config contains signing key without an ingress uri",
                config: Some(PublishConfig {
                    signing_private_key: Some(key_config.clone()),
                }),
                args: CacheArgs {
                    store_url: None,
                    catalog: None,
                    signing_private_key: None,
                },
                ingress_uri: None,
                expected: Ok(None),
            },
            TestCase {
                name: "error when catalog has ingress uri and no key supplied",
                config: Some(PublishConfig {
                    signing_private_key: None,
                }),
                args: CacheArgs {
                    store_url: None,
                    catalog: None,
                    signing_private_key: None,
                },
                ingress_uri: Some(url_response_str.to_string()),
                expected: Err(anyhow!(formatdoc! {"
                    A signing key is required to upload artifacts.

                    You can supply a signing key by either:
                    - Providing a path to a key with the '--signing-private-key' option.
                    - Setting it in the config via 'flox config --set publish.signing-key <path>'

                    Or you can publish without uploading artifacts via the '--metadata-only' option.
            "})),
            },
        ];

        let (mut flox, _temp_dir) = flox_instance();
        for tc in test_cases {
            reset_mocks(&mut flox.catalog_client, vec![Response::Publish(
                PublishResponse {
                    ingress_uri: tc.ingress_uri,
                },
            )]);
            match (
                tc.expected,
                merge_cache_options(&flox, "catalog_name", tc.config, tc.args).await,
            ) {
                (Ok(expected), Ok(actual)) => assert_eq!(actual, expected),
                (Err(e), Err(e2)) => assert_eq!(e.to_string(), e2.to_string()),
                (_, actual) => panic!("unexpected result {actual:?} for test case: {}", tc.name),
            }
        }
    }

    #[test]
    fn detects_default_publish_target() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello" 

            [build.hello]
            command = '''
                doesn't matter
            '''
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let target = Publish::get_publish_target(&manifest, &None).unwrap();
        assert_eq!(target, "hello");
    }

    #[test]
    fn error_when_no_publish_target_arg_no_builds() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello" 
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let res = Publish::get_publish_target(&manifest, &None);
        assert!(res.is_err());
    }

    #[test]
    fn error_when_no_publish_target_arg_multiple_builds() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello" 

            [build.hello]
            command = '''
                doesn't matter
            '''

            [build.hello2]
            command = '''
                doesn't matter
            '''
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let res = Publish::get_publish_target(&manifest, &None);
        assert!(res.is_err());
    }

    #[test]
    fn no_error_when_target_arg_supplied_multiple_builds() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello" 

            [build.hello]
            command = '''
                doesn't matter
            '''

            [build.hello2]
            command = '''
                doesn't matter
            '''
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let target = Publish::get_publish_target(
            &manifest,
            &Some(PublishTarget {
                target: "hello2".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(target, "hello2".to_string());
    }

    #[test]
    fn no_error_when_target_arg_supplied_one_build() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello" 

            [build.hello]
            command = '''
                doesn't matter
            '''
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let target = Publish::get_publish_target(
            &manifest,
            &Some(PublishTarget {
                target: "hello".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(target, "hello".to_string());
    }
}
