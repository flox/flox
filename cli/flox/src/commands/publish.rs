use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use bpaf::Bpaf;
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::models::manifest::typed::Manifest;
use flox_rust_sdk::providers::auth::Auth;
use flox_rust_sdk::providers::build::{COMMON_NIXPKGS_URL, PackageTarget, nix_expression_dir};
use flox_rust_sdk::providers::publish::{
    PublishProvider,
    Publisher,
    build_repo_err,
    check_build_metadata,
    check_environment_metadata,
    check_package_metadata,
};
use indoc::formatdoc;
use tracing::{debug, info_span, instrument};

use super::{DirEnvironmentSelect, dir_environment_select};
use crate::commands::build::{
    BaseCatalogUrlSelect,
    SystemOverride,
    base_catalog_url_select,
    base_nixpkgs_url_from_url_select,
    check_git_tracking_for_expression_builds,
    disallow_base_url_select_for_manifest_builds,
    packages_to_build,
    prefetch_expression_build_flake_ref,
    prefetch_flake_ref,
    system_override,
};
use crate::commands::{SHELL_COMPLETION_FILE, ensure_floxhub_token};
use crate::config::Config;
use crate::utils::errors::display_chain;
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

const PUBLISH_COMPLETION_POLL_INTERVAL_MILLIS: u64 = 2_000; // 1s
const PUBLISH_COMPLETION_TIMEOUT_MILLIS: u64 = 30 * 60 * 1_000; // 30 min

#[derive(Bpaf, Clone)]
pub struct Publish {
    #[bpaf(external(dir_environment_select), fallback(Default::default()))]
    environment: DirEnvironmentSelect,

    #[bpaf(external(cache_args))]
    cache: CacheArgs,

    /// Only publish the metadata of the package, and do not upload the artifact
    /// itself.
    ///
    /// With this option present, a signing key is not required.
    #[bpaf(long, hide)]
    metadata_only: bool,

    #[bpaf(external(base_catalog_url_select), optional)]
    base_catalog_url_select: Option<BaseCatalogUrlSelect>,

    #[bpaf(external(system_override))]
    system_override: SystemOverride,

    #[bpaf(external(publish_target), optional)]
    publish_target: Option<PublishTarget>,
}

#[derive(Debug, Bpaf, Clone, Default)]
struct CacheArgs {
    /// Specify the organization to which an artifact should be published to.
    /// Takes precedence over the default value of the user's GitHub handle.
    #[bpaf(short, long, argument("NAME"))]
    org: Option<String>,

    /// The private key to use in signing the package
    /// during upload.
    /// This is a local file path.
    /// This option is only necessary when using a Catalog Store not provided by
    /// Flox.
    /// Takes precedence over the value of `publish.signing_private_key` from
    /// 'flox config'.
    #[bpaf(long, argument("FILE"), complete_shell(SHELL_COMPLETION_FILE))]
    signing_private_key: Option<PathBuf>,
}

#[derive(Debug, Bpaf, Clone)]
struct PublishTarget {
    /// The package to publish.
    /// Possible values are all keys under the `build` attribute in the
    /// environment's `manifest.toml`.
    #[bpaf(positional("package"))]
    target: String,
}

/// Configuration options for the publish command
#[derive(Debug, Clone)]
struct PublishConfig {
    metadata_only: bool,
    cache_args: CacheArgs,
    base_catalog_url_select: Option<BaseCatalogUrlSelect>,
    system_override: SystemOverride,
}

impl Publish {
    pub async fn handle(self, config: Config, flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&flox, "Publish")?;
        environment_subcommand_metric!("publish", env);

        let publish_config = PublishConfig {
            metadata_only: self.metadata_only,
            cache_args: self.cache,
            base_catalog_url_select: self.base_catalog_url_select,
            system_override: self.system_override,
        };

        Self::publish(config, flox, env, self.publish_target, publish_config).await
    }

    fn get_publish_target(
        manifest: &Manifest,
        expression_dir: &Path,
        target_arg: Option<PublishTarget>,
    ) -> Result<PackageTarget> {
        match packages_to_build(
            manifest,
            expression_dir,
            &Vec::from_iter(target_arg.map(|arg| arg.target)),
        )?
        .as_slice()
        {
            [target] => Ok(target.clone()),
            [] => bail!("Cannot publish without a build specified"),
            _ => bail!("Must specify an artifact to publish"),
        }
    }

    #[instrument(name = "publish", skip_all, fields(package))]
    async fn publish(
        config: Config,
        mut flox: Flox,
        env: ConcreteEnvironment,
        package_arg: Option<PublishTarget>,
        publish_config: PublishConfig,
    ) -> Result<()> {
        // Fail as early as possible if the user isn't authenticated or doesn't
        // belong to an org with a catalog.
        let token = ensure_floxhub_token(&mut flox).await?.clone();
        let catalog_name = publish_config
            .cache_args
            .org
            .clone()
            .unwrap_or(token.handle().to_string());

        let path_env = match env {
            ConcreteEnvironment::Path(path_env) => path_env,
            ConcreteEnvironment::Managed(_) => {
                bail!("Cannot publish from an environment on FloxHub.")
            },
            ConcreteEnvironment::Remote(_) => {
                unreachable!("Cannot publish from a remote environment")
            },
        };

        // If the environment isn't locked, locking it will modify the lockfile,
        // which will mean the repo will have uncommitted changes.
        // Instead of locking and erroring later on, error now.
        let Some(lockfile) = path_env.existing_lockfile(&flox)? else {
            bail!(build_repo_err("Environment must be locked."));
        };
        let expression_dir = nix_expression_dir(&path_env);

        // Used for non building expressions and manifest builds
        prefetch_flake_ref(&COMMON_NIXPKGS_URL)?;

        let package = Self::get_publish_target(&lockfile.manifest, &expression_dir, package_arg)?;

        disallow_base_url_select_for_manifest_builds(
            [&package],
            publish_config.base_catalog_url_select.is_some(),
        )?;
        // Note: when publishsing an expression build,
        // this causes us to discover the containing git repo twice.
        // While slightly redundant it outweighs the complexity of reusing git instances.
        check_git_tracking_for_expression_builds([&package], &expression_dir)?;

        // Check the environment for appropriate state to build and publish
        let env_metadata = check_environment_metadata(&flox, &path_env)?;

        let selected_base_nixpkgs_url = base_nixpkgs_url_from_url_select(
            &flox,
            publish_config.base_catalog_url_select,
            Some(&env_metadata.lockfile),
        )
        .await?;

        prefetch_expression_build_flake_ref(
            [&package],
            &selected_base_nixpkgs_url.as_flake_ref()?,
        )?;

        let package_metadata = check_package_metadata(
            &selected_base_nixpkgs_url,
            env_metadata.toplevel_catalog_ref.as_ref(),
            package,
        )?;

        let auth = Auth::from_flox(&flox)?;
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        // Check that we can publish before building.
        let package_created = publish_provider
            .create_package_and_possibly_user_catalog(&flox.catalog_client, &catalog_name)
            .await?;

        subcommand_metric!(
            "publish",
            "has_expression_build" = publish_provider
                .package_metadata
                .package
                .kind()
                .is_expression_build(),
            "has_manifest_build" = publish_provider
                .package_metadata
                .package
                .kind()
                .is_manifest_build()
        );
        let build_metadata = check_build_metadata(
            &flox,
            &selected_base_nixpkgs_url,
            publish_config.system_override.system,
            &publish_provider.env_metadata,
            &publish_provider.package_metadata.package,
        )?;

        // CLI args take precedence over config
        let key_file = publish_config.cache_args.signing_private_key.or(config
            .flox
            .publish
            .as_ref()
            .and_then(|cfg| cfg.signing_private_key.clone()));

        debug!(
            "publishing package: {}",
            &publish_provider.package_metadata.package
        );
        match publish_provider
            .publish(
                &flox.catalog_client,
                &catalog_name,
                package_created,
                &build_metadata,
                key_file,
                publish_config.metadata_only,
            )
            .await
        {
            Ok(_) => {
                let span = info_span!(
                    "publish",
                    progress = "Waiting for confirmation of successful publish..."
                );
                {
                    // Using a block here instead of `span.in_scope()` because
                    // that's not an async context.
                    let _ = span.enter();
                    publish_provider
                        .wait_for_publish_completion(
                            &flox.catalog_client,
                            &build_metadata,
                            PUBLISH_COMPLETION_POLL_INTERVAL_MILLIS,
                            PUBLISH_COMPLETION_TIMEOUT_MILLIS,
                        )
                        .await
                        .context("Failed while waiting for publish confirmation")?;
                }
            },
            Err(e) => bail!("Failed to publish package: {}", display_chain(&e)),
        }
        message::updated(formatdoc! {"
            Package published successfully.

            Use 'flox install {catalog_name}/{package}' to install it.
            ", package = &publish_provider.package_metadata.package,});
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

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

        let target =
            Publish::get_publish_target(&manifest, Path::new("/no/expression/builds"), None)
                .unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild
            )
        );
    }

    #[test]
    fn error_when_no_publish_target_arg_no_builds() {
        let manifest_str = formatdoc! {r#"
            version = 1

            [install]
            hello.pkg-path = "hello"
        "#};
        let manifest = Manifest::from_str(&manifest_str).unwrap();
        let res = Publish::get_publish_target(&manifest, Path::new("/no/expression/builds"), None);
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
        let res = Publish::get_publish_target(&manifest, Path::new("/no/expression/builds"), None);
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
            Path::new("/no/expression/builds"),
            Some(PublishTarget {
                target: "hello2".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello2",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild
            )
        );
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
            Path::new("/no/expression/builds"),
            Some(PublishTarget {
                target: "hello".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild
            )
        );
    }
}
