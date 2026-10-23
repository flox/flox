use std::collections::BTreeSet;
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
    PackageTargetKind,
    PackageTargets,
    nix_expression_dir,
};
use flox_rust_sdk::providers::nix_auth::NixAuth;
use flox_rust_sdk::providers::publish::{
    LineageOverride,
    PublishError,
    PublishOutcome,
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
    PackageSystem,
    RegisteredSource,
    SourceLineageChange,
};
use indoc::formatdoc;
use nef_lock_catalog::{
    CatalogRef,
    NixFlakeref,
    PackageClosure,
    UPDATE_CATALOGS_COMMAND,
    scan_package,
};
use tracing::{debug, info_span, instrument, warn};

use super::{DirEnvironmentSelect, dir_environment_select};
use crate::commands::build::{
    BaseCatalogUrlSelect,
    SystemOverride,
    TargetChoice,
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
use crate::utils::dialog::{Confirm, Dialog};
use crate::utils::errors::display_chain;
use crate::utils::events::env_detail_from_concrete;
use crate::utils::message;
use crate::{environment_subcommand_metric, subcommand_metric};

const PUBLISH_COMPLETION_POLL_INTERVAL_MILLIS: u64 = 2_000; // 1s
const PUBLISH_COMPLETION_TIMEOUT_MILLIS: u64 = 30 * 60 * 1_000; // 30 min

fn emit_publish_warnings(outcome: &PublishOutcome, mut emit: impl FnMut(&str)) {
    for warning in &outcome.warnings {
        emit(&format!("{}: {}", warning.input, warning.message));
    }
}

/// Show an accepted publish's warnings, then wait for the publisher service
/// when it is responsible for ingesting artifacts (Publisher mode). NixCopy
/// and MetadataOnly submit NAR info directly, so there is nothing to wait for.
/// The warnings come first so a later polling failure cannot hide them.
async fn finish_accepted_publish(
    outcome: &PublishOutcome,
    emit_warning: impl FnMut(&str),
    wait_for_publisher: impl AsyncFnOnce() -> Result<()>,
) -> Result<()> {
    emit_publish_warnings(outcome, emit_warning);
    if outcome.needs_publisher_wait {
        wait_for_publisher().await?;
    }
    Ok(())
}

/// Format a source for the lineage prompt as its URL and bare branch name.
fn display_source(url: &str, ref_: Option<&str>) -> String {
    let ref_ = ref_.map(|ref_| ref_.strip_prefix("refs/heads/").unwrap_or(ref_));
    format!("{url} (ref {})", ref_.unwrap_or("unset"))
}

/// Ask to replace the registered source; declining cancels the publish.
async fn prompt_lineage_change(registered: &str, requested: &str) -> Result<(), PublishError> {
    message::warning(formatdoc! {
        "Package source repository or ref has changed.
        Registered source: {registered}
        Requested source: {requested}

        Changing the repository can affect which build other packages lock as an input.
        Where automated builds run, future builds use the newly registered source.
        Previously published builds remain stored; existing locks are unchanged.

        Continue only if the requested repository and ref are the intended source for
        this package going forward.",
    });
    let confirmed = Dialog {
        message: "Change the registered source and publish?",
        help_message: None,
        typed: Confirm {
            default: Some(false),
        },
    }
    .prompt()
    .await
    .map_err(|err| match err {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            PublishError::LineageChangeDeclined
        },
        err => PublishError::Catchall(format!("Could not confirm source change: {err}")),
    })?;
    if !confirmed {
        return Err(PublishError::LineageChangeDeclined);
    }
    Ok(())
}

async fn confirm_lineage_change(change: &SourceLineageChange) -> Result<bool, PublishError> {
    if !Dialog::can_prompt() {
        return Ok(false);
    }
    prompt_lineage_change(&change.registered, &change.requested).await?;
    Ok(true)
}

/// A declined confirmation is a deliberate cancel, not a failure.
fn publish_failure(err: PublishError) -> anyhow::Error {
    match err {
        err @ PublishError::LineageChangeDeclined => err.into(),
        err => anyhow!("Failed to publish package: {}", display_chain(&err)),
    }
}

/// Carries the Factory's per-build token into the build pod. Read by
/// [`factory_build_token_from_env`] and forwarded, unread, on the two
/// catalog-server calls `flox publish` makes.
const FACTORY_BUILD_TOKEN_VAR: &str = "_FLOX_FACTORY_BUILD_TOKEN";

/// Read the Factory build token from the process environment.
///
/// Called once per publish and threaded to the call sites that need it
/// (the dedup check and the publish body) rather than re-read. Unset,
/// empty, and non-UTF-8 all mean absent, and absent means the field is
/// omitted from both request bodies `flox publish` sends. The value is
/// opaque and never parsed, validated, or checked for a prefix. It is
/// not a credential either: it authenticates nothing and only names
/// which build a publish belongs to, so it is logged as-is.
fn factory_build_token_from_env() -> Option<String> {
    let token = std::env::var(FACTORY_BUILD_TOKEN_VAR)
        .ok()
        .filter(|s| !s.is_empty());
    if let Some(value) = token.as_deref() {
        tracing::info!(
            factory_build_token = value,
            "forwarding Factory build token"
        );
    }
    token
}

/// What the check-build preflight reports about this publish.
#[derive(Debug)]
enum Preflight {
    /// Publishing would replace this registered source.
    LineageChanged(RegisteredSource),
    /// The server confirmed this exact build (by closure identity) was
    /// already published. The caller should display provenance and skip the
    /// upload.
    AlreadyPublished(CheckBuildResponse),
    /// The build is new; proceed with the publish.
    New,
}

/// Classify a check-build response. A lineage change wins over a
/// duplicate: replacing the registration still needs a publish.
fn preflight(response: CheckBuildResponse) -> Preflight {
    match response {
        CheckBuildResponse {
            lineage_changed: true,
            registered_source: Some(registered),
            ..
        } => Preflight::LineageChanged(registered),
        response if response.already_published => Preflight::AlreadyPublished(response),
        _ => Preflight::New,
    }
}

/// Run the check-build preflight before the build.
///
/// Returns `None` when this exact build was already published, after
/// printing its provenance, and otherwise the lineage override the publish
/// carries. A lineage change is confirmed here, bound to the registration
/// the server reported, or refused when no prompt is possible. A failed
/// check is only an optimization lost: it warns and lets the publish
/// continue.
async fn run_preflight(
    client: &impl CatalogClientTrait,
    query: CheckBuildQuery<'_>,
    requested_source: &str,
) -> Result<Option<LineageOverride>, PublishError> {
    let response = match client.check_build_already_recorded(query).await {
        Ok(response) => response,
        Err(e) => {
            message::warning("Unable to check if already published — continuing with publish.");
            warn!(
                error = %e,
                "Dedup check failed, proceeding with publish"
            );
            return Ok(Some(LineageOverride::Refuse));
        },
    };
    match preflight(response) {
        Preflight::LineageChanged(registered) => {
            let registered_display = display_source(&registered.url, registered.ref_.as_deref());
            if !Dialog::can_prompt() {
                return Err(PublishError::CatalogError(
                    FloxhubClientError::SourceLineageChange(SourceLineageChange {
                        registered: registered_display,
                        requested: requested_source.to_owned(),
                    }),
                ));
            }
            prompt_lineage_change(&registered_display, requested_source).await?;
            Ok(Some(LineageOverride::Expected(registered)))
        },
        Preflight::AlreadyPublished(resp) => {
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
            Ok(None)
        },
        Preflight::New => Ok(Some(LineageOverride::Refuse)),
    }
}

/// This package's roots and transitive closure, projected from the lock its
/// build consumes, with a stale committed lock translated into an
/// actionable error. Only the committed lock can be stale: an ephemeral
/// lock is resolved from the same expressions the references were scanned
/// from.
fn project_for_publish(
    lock: &BuildLockGuard,
    references: &BTreeSet<CatalogRef>,
) -> Result<PackageClosure> {
    lock.build_lock().project_package(references).map_err(|err| {
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

    /// Allow replacing the package's registered source repository or ref.
    #[bpaf(long)]
    allow_lineage_change: bool,

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
    /// If omitted and there are multiple builds, prompts for one when
    /// running interactively, otherwise lists them.
    #[bpaf(positional("package"))]
    target: String,
}

/// Configuration options for the publish command
#[derive(Debug, Clone)]
struct PublishConfig {
    metadata_only: bool,
    allow_lineage_change: bool,
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
            allow_lineage_change: self.allow_lineage_change,
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
    ) -> Result<TargetChoice> {
        let mut targets = packages_to_build(
            manifest,
            expression_ref,
            &Vec::from_iter(target_arg.map(|arg| arg.target)),
        )?;
        match targets.len() {
            1 => Ok(TargetChoice::Resolved(targets.remove(0))),
            _ => Ok(TargetChoice::Ambiguous(targets)),
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
                Self::get_publish_target(&lockfile_manifest, &expression_ref_local, package_arg)?
                    .resolve("publish", "to publish")
                    .await?;

            // Note: when publishing an expression build,
            // this causes us to discover the containing git repo twice.
            // While slightly redundant it outweighs the complexity of reusing git instances.
            check_git_tracking_for_expression_builds([&package], &expression_dir_parent)?;
            package
        };

        disallow_base_url_select_for_manifest_builds(
            [&package],
            publish_config.base_catalog_url_select.as_ref(),
        )?;

        // Check the environment for appropriate state to build and publish
        let env_metadata = check_environment_metadata(&flox, &path_env)?;

        // Only an expression build needs a nixpkgs selected for it; a manifest
        // build uses the one its environment is locked to, which is also the
        // one it is recorded against.
        let selected = match package.kind().is_expression_build() {
            true => Some(
                base_nixpkgs_url_from_url_select(&flox, publish_config.base_catalog_url_select)
                    .await?,
            ),
            false => None,
        };

        let package_metadata = check_package_metadata(
            selected.as_ref(),
            env_metadata.toplevel_catalog_ref.as_ref(),
            package,
        )?;

        // The page the package is recorded against is the page it is built
        // against, so there is one value rather than two to keep in step.
        let selected_base_nixpkgs_url = package_metadata.base_catalog_ref.clone();

        prefetch_expression_build_flake_ref(
            [&package_metadata.package],
            &selected_base_nixpkgs_url.as_flake_ref()?,
        )?;

        let auth = NixAuth::from_flox(&flox)?;
        let publish_provider = PublishProvider::new(env_metadata, package_metadata, auth);

        // Prepare the local lock before creating any server package record.
        let catalog = &flox.floxhub_client;

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
        // CLI: a temporary builder file materialized from the committed
        // lock, or from fresh resolution for the NEF evals. Scanning is scoped
        // to the published expression — the
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
        let catalog_lock = BuildLockGuard::for_expression_builds(
            &flox.floxhub_client,
            path_env.dot_flox_path(),
            &lock_rel_paths,
            flox.retain_diagnostic_tempfiles,
        )
        .await?;

        // This package's roots and closure, projected from the lock the
        // build consumes. Knowable before any build runs, so a true
        // duplicate skips the build entirely.
        let closure = match &catalog_lock {
            Some(lock) => project_for_publish(lock, &references)?,
            None => PackageClosure::default(),
        };
        let package_created = publish_provider
            .create_package_and_possibly_user_catalog(catalog, &catalog_name)
            .await?;

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
        // Read once and forwarded, unread, on every catalog-server call this
        // publish makes.
        let factory_build_token = factory_build_token_from_env();

        let mut lineage_override = if publish_config.allow_lineage_change {
            LineageOverride::Unbound
        } else {
            LineageOverride::Refuse
        };
        // Explicit source replacement must reach publish even for an existing build.
        if let Some(system) = dedup_system
            && !publish_config.allow_lineage_change
        {
            // Check the target's own identity; dependency metadata cannot stand in.
            let dot_flox_dir = publish_provider
                .env_metadata
                .rel_expression_build_base_dir
                .to_string_lossy();
            let query = CheckBuildQuery {
                catalog_name: &catalog_name,
                package_name: publish_provider.package_metadata.package.name().as_ref(),
                source_url: &publish_provider.env_metadata.build_repo_meta.url,
                source_rev: &publish_provider.env_metadata.build_repo_meta.rev,
                nixpkgs_rev,
                system,
                locked_inputs: &closure.locked_inputs,
                factory_build_token: factory_build_token.as_deref(),
                direct_inputs: &closure.direct_inputs,
                source_ref: &publish_provider.env_metadata.build_repo_meta.ref_,
                dot_flox_dir: &dot_flox_dir,
            };
            let build_repo_meta = &publish_provider.env_metadata.build_repo_meta;
            let requested_source =
                display_source(build_repo_meta.url.as_str(), Some(&build_repo_meta.ref_));
            match run_preflight(&flox.floxhub_client, query, &requested_source)
                .await
                .map_err(publish_failure)?
            {
                Some(confirmed) => lineage_override = confirmed,
                None => return Ok(()),
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
        let outcome = publish_provider
            .publish(
                catalog,
                &catalog_name,
                package_created,
                &build_metadata,
                &closure,
                key_file,
                publish_config.metadata_only,
                factory_build_token.as_deref(),
                lineage_override,
                confirm_lineage_change,
            )
            .await
            .map_err(publish_failure)?;

        finish_accepted_publish(
            &outcome,
            |warning| message::warning(warning),
            async || {
                let span = info_span!(
                    "publish",
                    progress = "Waiting for confirmation of successful publish..."
                );
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
                    .context("Failed while waiting for publish confirmation")
            },
        )
        .await?;
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
    use flox_rust_sdk::providers::build::PackageTarget;
    use flox_rust_sdk::providers::build::test_helpers::prepare_empty_expressions_ref;
    use floxhub_client::PublishWarning;
    use indoc::indoc;

    use super::*;
    use crate::utils::catalog_lock::test_helpers::build_lock_guard_from_parts;

    #[test]
    fn accepted_publish_displays_advisory_input_warning() {
        let outcome = PublishOutcome {
            needs_publisher_wait: false,
            warnings: vec![PublishWarning {
                code: "input_lineage_changed".to_owned(),
                input: "dependency".to_owned(),
                message: "dependency moved to a newer source lineage".to_owned(),
            }],
        };
        let mut displayed = Vec::new();
        emit_publish_warnings(&outcome, |message| displayed.push(message.to_owned()));
        assert_eq!(displayed, [
            "dependency: dependency moved to a newer source lineage"
        ]);
    }

    #[tokio::test]
    async fn accepted_publish_warnings_stay_visible_when_polling_fails() {
        let outcome = PublishOutcome {
            needs_publisher_wait: true,
            warnings: vec![PublishWarning {
                code: "input_lineage_changed".to_owned(),
                input: "dependency".to_owned(),
                message: "dependency moved to a newer source lineage".to_owned(),
            }],
        };
        let mut displayed = Vec::new();
        let err = finish_accepted_publish(
            &outcome,
            |message| displayed.push(message.to_owned()),
            async || {
                Err(anyhow!("publisher timed out")
                    .context("Failed while waiting for publish confirmation"))
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            (displayed, format!("{err:#}")),
            (
                vec!["dependency: dependency moved to a newer source lineage".to_owned()],
                "Failed while waiting for publish confirmation: publisher timed out".to_owned()
            )
        );
    }

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
        let err = project_for_publish(&lock, &references)
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
            TargetChoice::Resolved(PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            ))
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
    fn ambiguous_when_no_publish_target_arg_multiple_builds() {
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
        let choice =
            Publish::get_publish_target(&manifest, prepare_empty_expressions_ref(), None).unwrap();
        let manifest_build = PackageTargetKind::ManifestBuild { sandbox: None };
        assert_eq!(
            choice,
            TargetChoice::Ambiguous(vec![
                PackageTarget::new_unchecked("hello", manifest_build.clone()),
                PackageTarget::new_unchecked("hello2", manifest_build),
            ])
        );
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
            TargetChoice::Resolved(PackageTarget::new_unchecked(
                "hello2",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            ))
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
            TargetChoice::Resolved(PackageTarget::new_unchecked(
                "hello",
                flox_rust_sdk::providers::build::PackageTargetKind::ManifestBuild { sandbox: None }
            ))
        );
    }

    // --- preflight unit tests ---

    fn make_check_response(
        already_published: bool,
        registered_source: Option<RegisteredSource>,
        lineage_changed: bool,
    ) -> CheckBuildResponse {
        CheckBuildResponse {
            already_published,
            lineage_changed,
            published_at: None,
            registered_source,
            source_rev: None,
            source_rev_date: None,
        }
    }

    /// Tests cannot prompt, so a lineage change stops the publish before
    /// the build.
    #[tokio::test]
    async fn run_preflight_acts_on_the_check_build_answer() {
        use std::collections::BTreeMap;

        use floxhub_client::FloxhubClient;
        use floxhub_client::client::test_helpers::client_config;
        use httpmock::MockServer;

        let registered = RegisteredSource {
            ref_: Some("refs/heads/main".to_owned()),
            url: "https://github.com/org/repo".to_owned(),
        };
        let requested_source = "https://github.com/org/fork (ref topic)";
        let lineage_change = SourceLineageChange {
            registered: "https://github.com/org/repo (ref main)".to_owned(),
            requested: requested_source.to_owned(),
        };
        let source_url = url::Url::parse("https://github.com/org/fork").unwrap();
        let locked_inputs = BTreeMap::new();
        for (status, body, expected) in [
            (
                200,
                serde_json::to_value(make_check_response(true, Some(registered.clone()), true))
                    .unwrap(),
                Err(lineage_change),
            ),
            (
                200,
                serde_json::to_value(make_check_response(false, None, true)).unwrap(),
                Ok(Some(LineageOverride::Refuse)),
            ),
            (
                200,
                serde_json::to_value(make_check_response(true, None, false)).unwrap(),
                Ok(None),
            ),
            (
                200,
                serde_json::to_value(make_check_response(false, None, false)).unwrap(),
                Ok(Some(LineageOverride::Refuse)),
            ),
            (
                500,
                serde_json::json!({"detail": "Internal Server Error"}),
                Ok(Some(LineageOverride::Refuse)),
            ),
        ] {
            let server = MockServer::start_async().await;
            let mock = server.mock(|when, then| {
                when.method("POST")
                    .path("/api/v1/catalog/catalogs/myorg/packages/mypkg/check-build");
                then.status(status).json_body(body.clone());
            });
            let client = FloxhubClient::new(client_config(server.base_url().as_str())).unwrap();
            let query = CheckBuildQuery {
                catalog_name: "myorg",
                package_name: "mypkg",
                source_url: &source_url,
                source_rev: "deadbeef",
                nixpkgs_rev: "cafebabe",
                system: PackageSystem::X8664Linux,
                locked_inputs: &locked_inputs,
                factory_build_token: None,
                direct_inputs: &[],
                source_ref: "refs/heads/topic",
                dot_flox_dir: ".flox",
            };

            let result = run_preflight(&client, query, requested_source)
                .await
                .map_err(|err| match err {
                    PublishError::CatalogError(FloxhubClientError::SourceLineageChange(change)) => {
                        change
                    },
                    err => panic!("unexpected error: {err}"),
                });

            mock.assert();
            assert_eq!(result, expected, "{status} {body}");
        }
    }

    #[test]
    fn display_source_names_the_bare_branch() {
        for (ref_, expected) in [
            (
                Some("refs/heads/main"),
                "https://github.com/org/repo (ref main)",
            ),
            (Some("main"), "https://github.com/org/repo (ref main)"),
            (None, "https://github.com/org/repo (ref unset)"),
        ] {
            assert_eq!(
                display_source("https://github.com/org/repo", ref_),
                expected
            );
        }
    }

    #[test]
    fn factory_build_token_from_env_unset_gives_none() {
        temp_env::with_var(FACTORY_BUILD_TOKEN_VAR, None::<&str>, || {
            assert_eq!(factory_build_token_from_env(), None);
        });
    }

    #[test]
    fn factory_build_token_from_env_empty_gives_none() {
        temp_env::with_var(FACTORY_BUILD_TOKEN_VAR, Some(""), || {
            assert_eq!(factory_build_token_from_env(), None);
        });
    }

    #[test]
    fn factory_build_token_from_env_set_gives_some() {
        temp_env::with_var(FACTORY_BUILD_TOKEN_VAR, Some("factory:abc123"), || {
            assert_eq!(
                factory_build_token_from_env(),
                Some("factory:abc123".to_string())
            );
        });
    }
}
