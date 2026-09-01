use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::{Context, Result, anyhow, bail};
use bpaf::Bpaf;
use flox_config::Config;
use flox_events::{CliEnvironmentPublishPayload, EventKind, EventsHub};
use flox_manifest::{Manifest, MigratedTypedOnly};
use flox_rust_sdk::flox::Flox;
use flox_rust_sdk::models::environment::{ConcreteEnvironment, Environment};
use flox_rust_sdk::providers::build::{
    COMMON_NIXPKGS_URL,
    PackageTarget,
    PackageTargetKind,
    PackageTargets,
    nix_expression_dir,
};
use flox_rust_sdk::providers::nix_auth::NixAuth;
use flox_rust_sdk::providers::publish::{
    PublishProvider,
    Publisher,
    build_repo_err,
    check_build_metadata,
    check_environment_metadata,
    check_package_metadata,
};
use floxhub_client::{
    CatalogClientTrait,
    CheckBuildQuery,
    CheckBuildResponse,
    FloxhubClientError,
    LockedInputEntry,
    PackageSystem,
};
use indoc::formatdoc;
use nef_lock_catalog::{CatalogRef, NixFlakeref, scan_package};
use tracing::{debug, info_span, instrument, warn};

use super::{DirEnvironmentSelect, dir_environment_select};
use crate::commands::build::{
    BaseCatalogUrlSelect,
    SystemOverride,
    UPDATE_CATALOGS_COMMAND,
    base_catalog_url_select,
    base_nixpkgs_url_from_url_select,
    check_git_tracking_for_expression_builds,
    disallow_base_url_select_for_manifest_builds,
    expression_rel_paths,
    packages_to_build,
    prefetch_expression_build_flake_ref,
    prefetch_flake_ref,
    system_override,
};
use crate::commands::{SHELL_COMPLETION_FILE, ensure_auth, needs_project_files_error};
use crate::utils::catalog_lock::BuildLockGuard;
use crate::utils::errors::display_chain;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

const PUBLISH_COMPLETION_POLL_INTERVAL_MILLIS: u64 = 2_000; // 1s
const PUBLISH_COMPLETION_TIMEOUT_MILLIS: u64 = 30 * 60 * 1_000; // 30 min

/// Outcome of the dedup pre-check against the catalog.
#[derive(Debug)]
enum DedupOutcome {
    /// The server confirmed this exact build (by closure identity) was already
    /// published. The caller should display provenance and skip the upload.
    AlreadyPublished(CheckBuildResponse),
    /// The build is new; proceed with the publish.
    New,
    /// The check request itself failed (network error, server error, etc.).
    /// A failed check must never block a publish — it is a best-effort
    /// optimisation only.
    CheckFailed(FloxhubClientError),
}

/// Map the raw check-build result to a [`DedupOutcome`].
///
/// An `Err` result (network failure, server error, etc.) maps to
/// `CheckFailed` rather than propagating: the dedup pre-check must never
/// prevent a legitimate publish.
fn dedup_outcome(result: Result<CheckBuildResponse, FloxhubClientError>) -> DedupOutcome {
    match result {
        Ok(resp) if resp.already_published => DedupOutcome::AlreadyPublished(resp),
        Ok(_) => DedupOutcome::New,
        Err(e) => DedupOutcome::CheckFailed(e),
    }
}

/// Run the dedup check and report whether the publish should stop because
/// this exact build was already published, printing its provenance when so.
/// A failed check warns and lets the publish continue.
async fn dedup_short_circuit(client: &impl CatalogClientTrait, query: CheckBuildQuery<'_>) -> bool {
    match dedup_outcome(client.check_build_already_recorded(query).await) {
        DedupOutcome::AlreadyPublished(resp) => {
            message::updated(formatdoc! {"
                Package already published.

                Originally published: {date}
                Source revision: {rev}
                ",
                date = resp
                    .published_at
                    .map_or_else(|| "unknown".to_string(), |d| d.to_string()),
                rev = resp.source_rev.unwrap_or_else(|| "unknown".to_string()),
            });
            true
        },
        DedupOutcome::New => false,
        DedupOutcome::CheckFailed(e) => {
            // A failed check must never block a publish; warn and continue.
            message::warning("Unable to check if already published — continuing with publish.");
            warn!(
                error = %e,
                "Dedup check failed, proceeding with publish"
            );
            false
        },
    }
}

/// The locked-input subset a publish submits, projected from the lock its
/// build consumes, with a stale committed lock translated into an
/// actionable error. Only the committed lock can be stale: an ephemeral
/// lock is resolved from the same expressions the references were scanned
/// from.
fn subset_for_publish(
    lock: &BuildLockGuard,
    references: &BTreeSet<CatalogRef>,
) -> Result<BTreeMap<String, LockedInputEntry>> {
    lock.build_lock().subset_direct(references).map_err(|err| {
        if lock.is_existing() {
            anyhow!(formatdoc! {"
                {err}
                Run '{update_catalogs}' to update '.flox/catalog.lock', then commit the file and retry 'flox publish'.",
                update_catalogs = UPDATE_CATALOGS_COMMAND})
        } else {
            anyhow::Error::from(err)
        }
    })
}

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
    pub async fn handle(self, config: Config, mut flox: Flox) -> Result<()> {
        let env = self
            .environment
            .detect_concrete_environment(&mut flox, "Publish")?;
        environment_subcommand_metric!("publish", env);
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentPublish(
            CliEnvironmentPublishPayload::new(env_detail_from_concrete(&flox, &env)),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let publish_config = PublishConfig {
            metadata_only: self.metadata_only,
            cache_args: self.cache,
            base_catalog_url_select: self.base_catalog_url_select,
            system_override: self.system_override,
        };

        Self::publish(config, flox, env, self.publish_target, publish_config).await
    }

    fn get_publish_target(
        manifest: &Manifest<MigratedTypedOnly>,
        expression_ref: &NixFlakeref,
        target_arg: Option<PublishTarget>,
    ) -> Result<PackageTarget> {
        match packages_to_build(
            manifest,
            expression_ref,
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
        let handle = ensure_auth(&mut flox).await?;
        let catalog_name = publish_config.cache_args.org.clone().unwrap_or(handle);

        let env_detail = env_detail_from_concrete(&flox, &env);
        let path_env = match env {
            ConcreteEnvironment::Path(path_env) => path_env,
            ConcreteEnvironment::Managed(managed) => {
                bail!(needs_project_files_error(&managed, "publish"))
            },
            ConcreteEnvironment::Remote(_) => {
                // guarded by DirEnvironmentSelect
                unreachable!("Cannot publish from a remote environment")
            },
        };

        // If the environment isn't locked, locking it will modify the lockfile,
        // which will mean the repo will have uncommitted changes.
        // Instead of locking and erroring later on, error now.
        let Some(lockfile) = path_env.existing_lockfile(&flox)? else {
            bail!(build_repo_err("Environment must be locked."));
        };

        // Used for non building expressions and manifest builds
        prefetch_flake_ref(&COMMON_NIXPKGS_URL)?;

        let lockfile_manifest = lockfile.migrated_manifest()?;
        let package = {
            let expression_dir_parent = path_env.dot_flox_path();
            let expression_ref_local = NixFlakeref::from_path(&expression_dir_parent)?;
            let package =
                Self::get_publish_target(&lockfile_manifest, &expression_ref_local, package_arg)?;

            // Note: when publishing an expression build,
            // this causes us to discover the containing git repo twice.
            // While slightly redundant it outweighs the complexity of reusing git instances.
            check_git_tracking_for_expression_builds([&package], &expression_dir_parent)?;
            package
        };

        disallow_base_url_select_for_manifest_builds(
            [&package],
            publish_config.base_catalog_url_select.is_some(),
        )?;

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

        let auth = NixAuth::from_flox(&flox)?;
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        // Check that we can publish before building.
        let catalog = &flox.floxhub_client;
        let package_created = publish_provider
            .create_package_and_possibly_user_catalog(catalog, &catalog_name)
            .await?;

        let has_expression_build = publish_provider
            .package_metadata
            .package
            .kind()
            .is_expression_build();
        let has_manifest_build = publish_provider
            .package_metadata
            .package
            .kind()
            .is_manifest_build();
        subcommand_metric!(
            "publish",
            "has_expression_build" = has_expression_build,
            "has_manifest_build" = has_manifest_build
        );
        if let Err(err) = EventsHub::global().record_event(EventKind::CliEnvironmentPublish(
            CliEnvironmentPublishPayload::new(env_detail)
                .with_build_kinds(has_expression_build, has_manifest_build)
                .with_manifest_version(
                    publish_provider
                        .env_metadata
                        .lockfile
                        .manifest_schema_version()
                        .to_string(),
                ),
        )) {
            debug!(error = %err, "Failed to record v2 event");
        }

        let system_override_inner = publish_config.system_override.into_inner();

        // The catalog references this package's expression makes, scanned
        // locally — no build, no network. Manifest builds resolve no catalog
        // inputs of their own.
        let references = match publish_provider.package_metadata.package.kind() {
            PackageTargetKind::ExpressionBuild(expression) => {
                scan_package(nix_expression_dir(&path_env), &expression.rel_file_path)?
            },
            PackageTargetKind::ManifestBuild { .. } => BTreeSet::new(),
        };

        // The lock this publish's build consumes, created up front by the
        // CLI: the committed .flox/catalog.lock exactly as found, or a fresh
        // ephemeral lock the package builder only passes through to the NEF
        // evals. Scanning is scoped to the published expression — the
        // scanner follows imports, so its references are exactly what its
        // eval looks up — except for a manifest build, whose `${pkg}`
        // references can pull in any of the project's expressions, so all of
        // them are covered. A project without expression builds needs no
        // lock at all.
        let lock_rel_paths: Vec<PathBuf> = match publish_provider.package_metadata.package.kind() {
            PackageTargetKind::ExpressionBuild(expression) => {
                vec![expression.rel_file_path.clone()]
            },
            PackageTargetKind::ManifestBuild { .. } => {
                let expression_ref_local = NixFlakeref::from_path(path_env.dot_flox_path())?;
                expression_rel_paths(
                    &PackageTargets::new(&lockfile_manifest, &expression_ref_local)?.all(),
                )
            },
        };
        let catalog_lock = match lock_rel_paths.is_empty() {
            true => None,
            false => Some(
                BuildLockGuard::new_existing_or_ephemeral(
                    &flox.floxhub_client,
                    path_env.dot_flox_path(),
                    &lock_rel_paths,
                )
                .await?,
            ),
        };

        // The locked-input subset this publish submits, projected from the
        // lock the build consumes. Knowable before any build runs, so a true
        // duplicate skips the build entirely.
        let locked_inputs = match &catalog_lock {
            Some(lock) => subset_for_publish(lock, &references)?,
            None => BTreeMap::new(),
        };

        // Dedup: ask the catalog server if this exact build has already been
        // published before paying for the upload — and, when the closure
        // identity is knowable without a build, before paying for the build
        // too. An unparsable system is left for the build to report; the
        // check is skipped rather than failed.
        let nixpkgs_rev = publish_provider.package_metadata.base_catalog_ref.rev();
        let nixpkgs_rev = nixpkgs_rev.as_deref().unwrap_or_else(|| {
            warn!(
                url = %publish_provider.package_metadata.base_catalog_ref,
                "could not extract nixpkgs rev from base catalog URL; \
                 dedup check will likely miss"
            );
            ""
        });
        // An explicit `--system` the catalog cannot represent would
        // otherwise silently skip the dedup check, pay for a full build, and
        // fail later with a generic error; reject it by name up front. The
        // native system always parses, so its fallible parse only guards
        // against the impossible.
        let dedup_system = match system_override_inner.as_deref() {
            Some(value) => Some(PackageSystem::from_str(value).map_err(|_| {
                anyhow!(
                    "'{value}' is not a system Flox can build for; expected 'aarch64-darwin', 'aarch64-linux', 'x86_64-darwin' or 'x86_64-linux'."
                )
            })?),
            None => PackageSystem::from_str(&flox.system).ok(),
        };
        if let Some(system) = dedup_system {
            let locked_inputs_query: HashMap<_, _> = locked_inputs.clone().into_iter().collect();
            let query = CheckBuildQuery {
                catalog_name: &catalog_name,
                package_name: publish_provider.package_metadata.package.name().as_ref(),
                source_url: &publish_provider.env_metadata.build_repo_meta.url,
                source_rev: &publish_provider.env_metadata.build_repo_meta.rev,
                nixpkgs_rev,
                system,
                locked_inputs: &locked_inputs_query,
            };
            if dedup_short_circuit(&flox.floxhub_client, query).await {
                return Ok(());
            }
        }

        let build_metadata = check_build_metadata(
            &flox,
            &selected_base_nixpkgs_url,
            system_override_inner,
            &publish_provider.env_metadata,
            &publish_provider.package_metadata.package,
            catalog_lock.as_ref().map(|lock| lock.path()),
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
        let catalog = &flox.floxhub_client;
        let needs_publisher_wait = match publish_provider
            .publish(
                catalog,
                &catalog_name,
                package_created,
                &build_metadata,
                &locked_inputs,
                key_file,
                publish_config.metadata_only,
            )
            .await
        {
            Ok(needs_wait) => needs_wait,
            Err(e) => bail!("Failed to publish package: {}", display_chain(&e)),
        };

        // Only poll when the external publisher service is responsible for
        // ingesting artifacts (Publisher mode). NixCopy and MetadataOnly
        // submit NAR info directly, so there is nothing to wait for.
        if needs_publisher_wait {
            let span = info_span!(
                "publish",
                progress = "Waiting for confirmation of successful publish..."
            );
            {
                // Using a block here instead of `span.in_scope()` because
                // that's not an async context.
                let _ = span.enter();
                let catalog = &flox.floxhub_client;
                publish_provider
                    .wait_for_publish_completion(
                        catalog,
                        &build_metadata,
                        PUBLISH_COMPLETION_POLL_INTERVAL_MILLIS,
                        PUBLISH_COMPLETION_TIMEOUT_MILLIS,
                    )
                    .await
                    .context("Failed while waiting for publish confirmation")?;
            }
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
    use flox_manifest::test_helpers::with_latest_schema;
    use flox_rust_sdk::providers::build::test_helpers::prepare_empty_expressions_ref;
    use indoc::indoc;

    use super::*;
    use crate::utils::catalog_lock::test_helpers::build_lock_guard_from_parts;

    /// A stale committed lock fails a publish naming both the uncovered
    /// reference and the recovery command.
    #[test]
    fn stale_committed_lock_subset_names_update_catalogs() {
        let expressions_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            expressions_dir.path().join("hello.nix"),
            "{ catalogs }: catalogs.myorg.hello",
        )
        .unwrap();
        let references = scan_package(expressions_dir.path(), "hello.nix").unwrap();

        let lock = build_lock_guard_from_parts(
            "/project/.flox/catalog.lock",
            nef_lock_catalog::BuildLock::default(),
            true,
        );
        let err = subset_for_publish(&lock, &references)
            .expect_err("an empty committed lock cannot cover the reference");
        let message = format!("{err:#}");
        assert!(
            message.contains(UPDATE_CATALOGS_COMMAND),
            "the hint must name the recovery command, got: {message}"
        );
        assert!(
            message.contains("myorg.hello"),
            "the uncovered reference must be named, got: {message}"
        );
    }

    #[test]
    fn detects_default_publish_target() {
        let manifest_contents = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"

            [build.hello]
            command = '''
                doesn't matter
            '''
        "#});
        let manifest = Manifest::parse_and_migrate(manifest_contents, None)
            .unwrap()
            .as_migrated_typed_only();

        let target =
            Publish::get_publish_target(&manifest, prepare_empty_expressions_ref(), None).unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            )
        );
    }

    #[test]
    fn error_when_no_publish_target_arg_no_builds() {
        let manifest_contents = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"
        "#});
        let manifest = Manifest::parse_and_migrate(manifest_contents, None)
            .unwrap()
            .as_migrated_typed_only();
        let res = Publish::get_publish_target(&manifest, prepare_empty_expressions_ref(), None);
        assert!(res.is_err());
    }

    #[test]
    fn error_when_no_publish_target_arg_multiple_builds() {
        let manifest_contents = with_latest_schema(indoc! {r#"
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
        "#});
        let manifest = Manifest::parse_and_migrate(manifest_contents, None)
            .unwrap()
            .as_migrated_typed_only();
        let res = Publish::get_publish_target(&manifest, prepare_empty_expressions_ref(), None);
        assert!(res.is_err());
    }

    #[test]
    fn no_error_when_target_arg_supplied_multiple_builds() {
        let manifest_contents = with_latest_schema(indoc! {r#"
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
        "#});
        let manifest = Manifest::parse_and_migrate(manifest_contents, None)
            .unwrap()
            .as_migrated_typed_only();
        let target = Publish::get_publish_target(
            &manifest,
            prepare_empty_expressions_ref(),
            Some(PublishTarget {
                target: "hello2".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello2",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            )
        );
    }

    #[test]
    fn no_error_when_target_arg_supplied_one_build() {
        let manifest_contents = with_latest_schema(indoc! {r#"
            [install]
            hello.pkg-path = "hello"

            [build.hello]
            command = '''
                doesn't matter
            '''
        "#});
        let manifest = Manifest::parse_and_migrate(manifest_contents, None)
            .unwrap()
            .as_migrated_typed_only();
        let target = Publish::get_publish_target(
            &manifest,
            prepare_empty_expressions_ref(),
            Some(PublishTarget {
                target: "hello".to_string(),
            }),
        )
        .unwrap();
        assert_eq!(
            target,
            PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            )
        );
    }

    // --- dedup_outcome unit tests ---

    fn make_check_response(already_published: bool) -> CheckBuildResponse {
        CheckBuildResponse {
            already_published,
            published_at: None,
            source_rev: None,
            source_rev_date: None,
        }
    }

    #[test]
    fn dedup_outcome_already_published_passes_response_through() {
        let input = make_check_response(true);
        let expected = input.clone();
        let DedupOutcome::AlreadyPublished(got) = dedup_outcome(Ok(input)) else {
            panic!("expected AlreadyPublished variant");
        };
        assert_eq!(got, expected);
    }

    #[test]
    fn dedup_outcome_not_published_maps_to_new() {
        let resp = make_check_response(false);
        assert!(matches!(dedup_outcome(Ok(resp)), DedupOutcome::New));
    }

    #[test]
    fn dedup_outcome_err_maps_to_check_failed() {
        let result: Result<CheckBuildResponse, FloxhubClientError> =
            Err(FloxhubClientError::Other("network error".to_string()));
        assert!(matches!(
            dedup_outcome(result),
            DedupOutcome::CheckFailed(_)
        ));
    }
}
