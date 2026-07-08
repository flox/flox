use std::collections::HashMap;
use std::ffi::OsStr;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;
use std::thread::ScopedJoinHandle;
use std::{env, fmt};

use flox_core::activate::mode::ActivateMode;
use flox_core::canonical_path::CanonicalPath;
use flox_manifest::ManifestError;
use flox_manifest::interfaces::{AsLatestSchema, PackageLookup};
use flox_manifest::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
    Lockfile,
    PackageToList,
};
use flox_manifest::parsed::latest::SelectedOutputs;
use floxhub_client::{CatalogClientTrait, FloxhubClientError, StoreInfo};
use pollster::FutureExt as _;
use rsevents_extra::Semaphore;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{Span, debug, info_span, instrument, trace, warn};

use super::nix::nix_base_command;
use super::nix_auth::{AuthError, AuthProvider};
use crate::data::System;
use crate::models::nix_plugins::NIX_PLUGINS;
use crate::providers::nix_auth::{catalog_auth_to_envs, store_needs_auth};
use crate::utils::CommandExt;

static BUILDENV_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILDENV_NIX")
        .unwrap_or_else(|_| env!("FLOX_BUILDENV_NIX").to_string())
        .into()
});

/// Prefix of locked_url of catalog packages that are from the nixpkgs base-catalog.
/// This url was meant to serve as a flake reference to the Flox hosted mirror of nixpkgs,
/// but is both ill formatted and does not provide the necessary overrides
/// to allow evaluating packages without common evaluation checks, such as unfree and broken.
const NIXPKGS_CATALOG_URL_PREFIX: &str = "https://github.com/flox/nixpkgs?rev=";

/// Returns `true` if `path` is accessible on the local filesystem.
///
/// Uses `metadata()` rather than `Path::exists()` so callers can distinguish
/// "does not exist" from "permission denied" if needed in future — the two are
/// equivalent for Nix store paths owned by the Nix daemon.
fn path_is_present(path: &str) -> bool {
    std::fs::metadata(path).is_ok()
}

/// The base flake reference invoking the `flox-nixpkgs` fetcher.
/// This is a bridge to the Flox hosted mirror of nixpkgs flake,
/// which enables building packages without common evaluation checks,
/// such as unfree and broken.
const FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE: &str = "flox-nixpkgs:v0/flox";

/// Name to use in error messages when a package can't be downloaded from
/// `cache.nixos.org` as a fallback for other locations.
const LOCATION_FALLBACK_NAME: &str = "base catalog";

///A collection of failed attempts to download a package from specific sources.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadAttempts(pub Vec<DownloadAttempt>);

impl Display for DownloadAttempts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, attempt) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n\n")?;
            }
            write!(f, "{attempt}")?;
        }
        Ok(())
    }
}

/// A failed attempt to download a package from a specific source.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadAttempt {
    pub location: String,
    pub error: String,
}

impl Display for DownloadAttempt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:\n{}",
            self.location,
            indent::indent_all_by(2, self.error.trim_end())
        )
    }
}

#[derive(Debug, Error)]
pub enum BuildEnvError {
    #[error("Failed to realise '{install_id}':\n{message}")]
    Realise2 { install_id: String, message: String },

    #[error(transparent)]
    Manifest(#[from] ManifestError),

    /// An error that occurred while composing the environment.
    /// I.e. `nix build` returned with a non-zero exit code.
    /// The error message is the stderr of the `nix build` command.
    // TODO: this requires to capture the stderr of the `nix build` command
    // or essentially "tee" it if we also want to forward the logs to the user.
    // At the moment, the "interesting" logs
    // are emitted by the `realise` portion of the build.
    // So in the interest of initial simplicity
    // we can defer forwarding the nix build logs and capture output with [Command::output].
    #[error("Failed to construct environment: {0}")]
    Build(String),

    #[error(
        "Lockfile is not compatible with the current system\n\
        Supported systems: {0}", systems.join(", "))]
    LockfileIncompatible { systems: Vec<String> },

    /// An error that occurred while creating GC roots via `create_gc_root_in`.
    #[error("Failed to link environment: {0}")]
    Link(String),

    /// An error that occurred while calling the client
    #[error("Unexpected error calling the catalog client")]
    CatalogError(#[source] FloxhubClientError),

    /// An error that occurred while accessing the cache
    #[error("Unexpected error accessing cache: {0}")]
    CacheError(String),

    /// An error that occurred while calling nix build.
    #[error("Failed to call 'nix build'")]
    CallNixBuild(#[source] std::io::Error),

    /// An error that occurred while deserializing the output of the `nix build` command.
    #[error("Failed to deserialize 'nix build' output:\n{output}\nError: {err}")]
    ReadOutputs {
        output: String,
        err: serde_json::Error,
    },

    #[error(
        "Can't find download location for package '{0}'.\nYou may not be authenticated or package may have been deleted.\nTry logging in with 'flox auth login'"
    )]
    NoPackageStoreLocation(String),
    #[error("Couldn't download package '{install_id}' from the following locations\n\n{attempts}")]
    BuildPublishedPackage {
        install_id: String,
        attempts: DownloadAttempts,
    },

    /// A custom package has been uploaded, but the current user hasn't configured
    /// a trusted public key that matches a signature of this package.
    #[error(
        "Package '{0}' is not signed by a trusted key.\n\
        See https://flox.dev/docs/customer/signing-keys/ for more information."
    )]
    UntrustedPackage(String),

    #[error("authentication error")]
    Auth(#[source] AuthError),

    /// An error occurred while performing nix copy
    /// The contained string should be stderr, which may be a bit too much
    /// detail,
    /// but it will allow debugging for now.
    #[error("couldn't download package:\n{0}")]
    NixCopyError(String),

    /// An unhandled condition was encountered in the lockfile.  One example is
    /// a package that is expected to be a base catalog package but the
    /// lockfile appears to be a custom package or vice versa.
    #[error("encountered an error interpreting the lockfile: {0}")]
    LockfileContents(String),

    /// A catch-all error variant for rare situations
    #[error("{0}")]
    Other(String),

    /// Store paths were unavailable after all materialisation retry attempts.
    #[error(
        "Store paths were unavailable after {attempts} materialisation attempts.\n\
        This is most likely caused by a concurrent garbage collection run.\n\
        If a garbage collection is in progress, wait for it to finish and retry.\n\
        Missing paths: {paths}"
    )]
    MaterialisationFailed { attempts: usize, paths: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct BuildEnvOutputs {
    pub dev: BuiltStorePath,
    pub run: BuiltStorePath,
    /// A map of additional built store paths.
    /// These are the runtime environments for each manifest build.
    /// The main consumer of this is [super::build::FloxBuildMk].
    // todo: nest additional built paths for manifest builds
    #[serde(flatten)]
    pub manifest_build_runtimes: HashMap<String, BuiltStorePath>,
}

impl BuildEnvOutputs {
    /// Returns the built environment path for an activation mode.
    pub fn for_mode(self, mode: &ActivateMode) -> BuiltStorePath {
        match mode {
            ActivateMode::Dev => self.dev,
            ActivateMode::Run => self.run,
        }
    }
}

#[derive(
    Debug, Clone, Serialize, Deserialize, PartialEq, Eq, derive_more::Deref, derive_more::AsRef,
)]
pub struct BuiltStorePath(PathBuf);

pub trait BuildEnv {
    fn build(
        &self,
        client: &impl CatalogClientTrait,
        lockfile: &Path,
        service_config_path: Option<PathBuf>,
        out_link_prefix: Option<&Path>,
    ) -> Result<BuildEnvOutputs, BuildEnvError>;
}

pub struct BuildEnvNix<A> {
    auth: A,
}

/// Build the base `nix build` command shared by all store-path operations.
///
/// Enables impure language features, applies the store URL override from
/// `_FLOX_NIX_STORE_URL`, and requests build logs.
fn base_command() -> Command {
    let mut nix_build_command = nix_base_command();
    // allow impure language features such as `builtins.storePath`,
    // and use the auto store (which is used by the preceding `realise` command)
    // TODO: formalize this in a config file,
    // and potentially disable other user configs (allowing specific overrides)
    nix_build_command.args(["--option", "pure-eval", "false"]);
    apply_nix_store_url(&mut nix_build_command);
    // we generally want to see more logs (we can always filter them out)
    nix_build_command.arg("--print-build-logs");

    nix_build_command
}

/// Fetch the given store paths via `nix build`, downloading from configured
/// substituters or building from source if needed.
///
/// When `out_link` is `Some(prefix)`, passes `--out-link <prefix>` so the
/// downloaded paths are registered as GC roots under that prefix.
/// When `out_link` is `None`, passes `--no-link` instead.
///
/// Returns `true` if all paths were fetched successfully, `false` otherwise.
pub fn substitute_store_paths(
    paths: impl IntoIterator<Item = impl AsRef<OsStr>>,
    out_link: Option<&Path>,
) -> Result<bool, BuildEnvError> {
    let paths: Vec<_> = paths.into_iter().collect();

    let mut cmd = base_command();
    cmd.arg("build");
    match out_link {
        Some(prefix) => {
            cmd.arg("--out-link").arg(prefix);
        },
        None => {
            cmd.arg("--no-link");
        },
    }
    cmd.args(paths);

    debug!(cmd=%cmd.display(), "trying to fetch store paths");

    let output = cmd.output().map_err(BuildEnvError::CallNixBuild)?;
    let success = output.status.success();
    if !success {
        debug!(
            stderr = %String::from_utf8_lossy(&output.stderr),
            "store path fetch failed"
        );
    }
    Ok(success)
}

/// Build a catalog package from source when the binary cache does not have it.
///
/// Constructs the `flox-nixpkgs` flake installable from `locked_url` and
/// `attr_path`, then invokes `nix build` with the Nix plugins that allow
/// unfree and broken packages.
///
/// When `gc_root_prefix` is `Some(p)`, passes `--out-link <p>` so the build
/// output is registered as a GC root. When `None`, passes `--no-link`.
///
/// Returns `Err(BuildEnvError::Realise2)` if the build fails.
pub fn build_catalog_pkg_from_source(
    locked_url: &str,
    attr_path: &str,
    system: &str,
    unfree: Option<bool>,
    broken: Option<bool>,
    gc_root_prefix: Option<&Path>,
) -> Result<(), BuildEnvError> {
    // Transform the locked URL: strip the nixpkgs GitHub prefix and replace
    // it with the flox-nixpkgs fetcher reference so that evaluation checks
    // (allowUnfree, allowBroken) are controlled by manifest options rather
    // than Nix's built-in guards.
    let flake_ref = if let Some(rev) = locked_url.strip_prefix(NIXPKGS_CATALOG_URL_PREFIX) {
        format!("{FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE}/{rev}")
    } else {
        return Err(BuildEnvError::LockfileContents(format!(
            "Locked package '{}' is a base catalog package, but the locked url '{}' does not start with the expected prefix '{}'",
            attr_path, locked_url, NIXPKGS_CATALOG_URL_PREFIX
        )));
    };

    // Build all outputs (^*) from legacyPackages.<system>.<attr_path>.
    let installable = format!("{flake_ref}#legacyPackages.{system}.{attr_path}^*");

    let reason = match (unfree, broken) {
        (Some(true), _) => " (unfree license)",
        (_, Some(true)) => " (upstream build marked as broken)",
        _ => "",
    };

    let _span = info_span!(
        "build_catalog_pkg_from_source",
        progress = format!("Building '{attr_path}' from source{reason}")
    )
    .entered();

    let mut cmd = base_command();
    cmd.args(["--option", "extra-plugin-files", &*NIX_PLUGINS]);
    cmd.arg("build");
    cmd.arg("--no-write-lock-file");
    cmd.arg("--no-update-lock-file");
    cmd.args(["--option", "pure-eval", "true"]);
    match gc_root_prefix {
        Some(prefix) => {
            cmd.arg("--out-link").arg(prefix);
        },
        None => {
            cmd.arg("--no-link");
        },
    }
    cmd.arg(&installable);

    debug!(%installable, cmd=%cmd.display(), "building catalog package from source");

    let output = cmd.output().map_err(BuildEnvError::CallNixBuild)?;
    if output.status.success() {
        return Ok(());
    }
    Err(BuildEnvError::Realise2 {
        install_id: attr_path.to_string(),
        message: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

/// Download store paths from custom catalog store locations.
///
/// Tries each `StoreInfo` location for the given store paths in order.
/// Publisher auth (AWS env vars) is used when a location carries `auth`;
/// FloxHub netrc auth is used when the URL needs auth and no publisher
/// auth is present. Falls back to the base substituters (cache.nixos.org)
/// if all custom locations fail — some custom packages are unmodified
/// nixpkgs re-uploads that are available there.
///
/// # Parameters
/// - `install_id`: install ID used in error messages (e.g. `"mycat/vim"`)
/// - `attr_path`: attribute path used in span labels (e.g. `"vim"`)
/// - `no_netrc_is_error`: `true` when no FloxHub token was present at auth
///   setup time; a netrc-authenticated location failure should be surfaced
///   rather than silently skipped
/// - `maybe_netrc_path`: path to a temporary netrc file for FloxHub token
///   auth; `None` when the user is not authenticated
///
/// # Concurrency
/// This function issues one `nix copy` per location attempt with no internal
/// concurrency limit. Callers that invoke it in parallel loops must provide
/// their own concurrency guard (e.g. a `Semaphore`).
pub fn copy_from_custom_catalog_locations(
    store_paths: &[String],
    install_id: &str,
    attr_path: &str,
    store_locations: &HashMap<String, Vec<StoreInfo>>,
    no_netrc_is_error: bool,
    maybe_netrc_path: Option<&Path>,
) -> Result<(), BuildEnvError> {
    // The way the data structures are written it's possible to have
    // different package outputs come from different store locations. That's
    // not *really* possible though, so we're going to grab the store
    // locations for an arbitrary output and ASSUME that they're
    // representative for all outputs.
    let first_path = store_paths.first().ok_or_else(|| {
        BuildEnvError::NixCopyError(format!("no store paths provided for '{install_id}'"))
    })?;

    let locations = store_locations
        .get(first_path)
        .ok_or_else(|| BuildEnvError::NoPackageStoreLocation(install_id.to_string()))?;

    let span = info_span!(
        "substitute custom catalog package",
        progress = format!("Downloading '{attr_path}'")
    );
    let _span_guard = span.enter();

    let mut auth_error: Option<BuildEnvError> = None;
    let mut download_attempts = DownloadAttempts(vec![]);
    let mut any_location_succeeded = false;

    for location in locations {
        let mut copy_command = nix_base_command();
        let location_url = match &location.url {
            Some(url) => url,
            None => {
                return Err(BuildEnvError::NixCopyError(format!(
                    "missing store location URL for package '{install_id}'"
                )));
            },
        };
        copy_command.arg("copy");
        // auth.is_some() corresponds to Publisher store type
        // auth.is_none() corresponds to NixCopy
        // TODO: this is turning into spaghetti and would be good to refactor,
        // but this code isn't very stable so hold off for now.
        if let Some(auth) = &location.auth {
            copy_command.envs(catalog_auth_to_envs(auth).map_err(BuildEnvError::Auth)?);
        } else {
            // We don't need a floxhub token for the NixOS public cache
            if store_needs_auth(location_url) {
                if let Some(ref netrc_path) = maybe_netrc_path {
                    copy_command.arg("--netrc-file").arg(netrc_path);
                } else if no_netrc_is_error {
                    return Err(BuildEnvError::Auth(AuthError::NoToken));
                }
            }
        }
        copy_command.arg("--from").arg(location_url);
        for sp in store_paths {
            copy_command.arg(sp);
        }
        // Log at trace: the command includes --netrc-file <path> when netrc
        // auth is used; the path points to a temp file containing the token.
        trace!(cmd=%copy_command.display(), "trying to copy published package");
        let output = copy_command
            .output()
            .map_err(|e| BuildEnvError::CacheError(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("because it lacks a signature by a trusted key") {
                return Err(BuildEnvError::UntrustedPackage(install_id.to_string()));
            }
            // We're expecting errors for netrc type auth, but not for
            // catalog provided auth.
            if location.auth.is_some() {
                auth_error = Some(BuildEnvError::NixCopyError(stderr.to_string()));
            }
            download_attempts.0.push(DownloadAttempt {
                location: location_url.clone(),
                error: stderr.to_string(),
            });
            debug!(%attr_path, %location_url, %stderr, "failed to copy custom package from store");
        } else {
            debug!(%attr_path, %location_url, "successfully copied custom package from store");
            any_location_succeeded = true;
            break;
        }
    }

    // Consider the package not found if (1) we never had any locations to
    // download from in the first place, or (2) we did have locations to
    // download from and we failed to find the package in any of them.
    let not_found_in_custom_catalogs = locations.is_empty() || !any_location_succeeded;

    // Some custom packages are just re-uploads of stuff in nixpkgs with
    // no modifications, so the package may be found in `cache.nixos.org`.
    // Fall back to attempting to download from the NixOS cache.
    if not_found_in_custom_catalogs {
        let ok = substitute_store_paths(store_paths, None)
            .map_err(|e| BuildEnvError::NixCopyError(e.to_string()))?;
        if !ok {
            if locations.is_empty() {
                return Err(BuildEnvError::NoPackageStoreLocation(
                    install_id.to_string(),
                ));
            }
            download_attempts.0.push(DownloadAttempt {
                location: LOCATION_FALLBACK_NAME.to_string(),
                error: "substitution from base catalog failed".to_string(),
            });
            return Err(BuildEnvError::BuildPublishedPackage {
                install_id: install_id.to_string(),
                attempts: download_attempts,
            });
        }
    }

    // If a publisher-auth location failed but a later location succeeded (or
    // the public-cache fallback succeeded), auth_error is set but the download
    // completed. Return Ok — the caller has its store paths.
    let _ = auth_error;
    Ok(())
}

impl<A> BuildEnvNix<A>
where
    A: AuthProvider,
{
    pub fn new(auth: A) -> BuildEnvNix<A> {
        BuildEnvNix { auth }
    }

    /// Realise all store paths of packages that are installed to the environment,
    /// for the given system.
    /// This goes through all packages in the lockfile and realises them with
    /// the appropriate method for the package type.
    ///
    /// See the individual realisation functions for more details.
    fn realise_lockfile(
        &self,
        client: &impl CatalogClientTrait,
        lockfile: &Lockfile,
        system: &System,
    ) -> Result<(), BuildEnvError> {
        let mut base_catalog_pkgs = vec![];
        let mut custom_catalog_pkgs = vec![];
        let mut flake_pkgs = vec![];
        let mut store_path_pkgs = vec![];

        let complete_migrated_manifest = lockfile.migrated_manifest()?;
        let manifest = complete_migrated_manifest.as_latest_schema();

        for package in lockfile.packages.iter() {
            if package.system() != system {
                continue;
            }

            // Look up the package entry in the manifest using the install_id
            let install_id = package.install_id();
            let manifest_package =
                manifest.pkg_descriptor_with_id(install_id).ok_or_else(|| {
                    BuildEnvError::LockfileContents(format!(
                        "Could not find package with install_id '{install_id}' in manifest"
                    ))
                })?;

            match package {
                LockedPackage::Catalog(pkg) => {
                    if manifest_package.is_from_custom_catalog() {
                        custom_catalog_pkgs.push(pkg);
                    } else {
                        base_catalog_pkgs.push(pkg);
                    }
                },
                LockedPackage::Flake(pkg) => flake_pkgs.push(pkg),
                LockedPackage::StorePath(pkg) => store_path_pkgs.push(pkg),
            }
        }

        let max_parallel_downloads: Option<u16> = std::env::var("FLOX_MAX_PARALLEL_DOWNLOADS")
            .ok()
            .and_then(|value| value.parse::<u16>().ok());
        let semaphore = if let Some(max_par) = max_parallel_downloads {
            Semaphore::new(max_par, max_par)
        } else {
            Semaphore::new(20, 20)
        };

        let span = Span::current();

        // Only query store info if we have custom catalog packages and they
        // aren't already on the system.
        let all_custom_pkg_store_paths = custom_catalog_pkgs
            .iter()
            .flat_map(|pkg| pkg.outputs.values().map(|sp| sp.to_string()))
            .collect::<Vec<_>>();
        let all_custom_catalog_packages_valid = all_custom_pkg_store_paths
            .iter()
            .all(|sp| path_is_present(sp.as_str()));
        let store_locations = if all_custom_catalog_packages_valid {
            None
        } else {
            let store_locations = client
                .get_store_info(all_custom_pkg_store_paths)
                .block_on()
                .map_err(BuildEnvError::CatalogError)?;
            Some(store_locations)
        };

        // We optimistically try to create a netrc file but don't bail on error
        // in case the user doesn't need it. The `no_netrc_is_error` indicates
        // that there was an authentication error. We're doing this weird dance
        // because the thread-safety (specifically, the lack of it) of the auth
        // provider makes it a real pain to pass it into other threads. It makes
        // life *much* easier to do some of this ahead of time.
        let no_netrc_is_error = self.auth.token().is_none();
        // Hold the TempPath alive for the duration of the build; dropping it
        // would delete the underlying file before nix copy can read it.
        let netrc_guard = self.auth.try_create_netrc();
        let borrowed_netrc_path: Option<&Path> = netrc_guard.as_deref();

        std::thread::scope(|s| {
            let mut thread_handles = vec![];

            // Substitute all base catalog and store path packages in a single
            // batched nix build invocation, running in parallel with any custom
            // catalog downloads below.
            let batch_handle = s.spawn(|| {
                Self::realise_base_and_store_batch(
                    &base_catalog_pkgs,
                    &store_path_pkgs,
                    span.clone(),
                    &semaphore,
                )
            });
            thread_handles.push(batch_handle);

            // Custom catalog packages run in parallel with the batch above.
            if let Some(ref store_locations) = store_locations {
                for pkg in custom_catalog_pkgs.iter() {
                    // Check if we already have the store paths for this package via stat().
                    if pkg.outputs.values().all(|p| path_is_present(p.as_str())) {
                        continue;
                    }
                    let handle = s.spawn(|| {
                        Self::realise_single_custom_catalog_pkg(
                            pkg,
                            store_locations,
                            no_netrc_is_error,
                            borrowed_netrc_path,
                            span.clone(),
                            &semaphore,
                        )
                    });
                    thread_handles.push(handle);
                }
            }

            join_realise_results(thread_handles)
        })?;

        // Intentionally build flakes one at a time. We're not worried about
        // slowing down the build by oversubscribing the CPU so much as we're
        // worried about potentially running out of memory if we end up building
        // multiple things from source at a single time.
        for flake in flake_pkgs.iter() {
            Self::realise_flake(flake)?;
        }

        Ok(())
    }

    /// Tries to substitute a single custom catalog package given store info
    /// locations that have been looked up ahead of time.
    ///
    /// Some store info locations will require authentication, but the store
    /// paths for this package may be available at a different location that
    /// doesn't require authentication, so we authenticate lazily as that may
    /// fail.
    ///
    /// It's also possible that a user has simply published an unmodified
    /// package that exists in nixpkgs. To cover that case there is a fallback
    /// that will attempt to substitute from cache.nixos.org if substituting
    /// from the provided store info locations fails.
    ///
    /// Delegates to `copy_from_custom_catalog_locations`; adds the semaphore
    /// guard and parent-span context needed for concurrent environment builds.
    fn realise_single_custom_catalog_pkg(
        locked_pkg: &LockedPackageCatalog,
        store_locations: &HashMap<String, Vec<StoreInfo>>,
        no_netrc_is_error: bool,
        maybe_netrc_path: Option<&Path>,
        parent_span: Span,
        semaphore: &Semaphore,
    ) -> Result<(), BuildEnvError> {
        let _sem_guard = semaphore.wait();
        // Enter parent span so the child span in copy_from_custom_catalog_locations
        // inherits the correct tracing hierarchy.
        let _parent_guard = parent_span.enter();
        let store_paths: Vec<String> = locked_pkg.outputs.values().cloned().collect();
        copy_from_custom_catalog_locations(
            &store_paths,
            &locked_pkg.install_id,
            &locked_pkg.attr_path,
            store_locations,
            no_netrc_is_error,
            maybe_netrc_path,
        )
    }

    /// Substitute all base catalog and store path packages in a single
    /// `nix build --no-link --keep-going` invocation.
    ///
    /// `--keep-going` ensures that as many paths as possible are fetched even
    /// when some are unavailable, so the per-package fallback below only needs
    /// to handle packages that genuinely failed. On partial failure the
    /// fallback runs per-package to surface accurate `Realise2` errors with
    /// install IDs and messages.
    fn realise_base_and_store_batch(
        base_catalog_pkgs: &[&LockedPackageCatalog],
        store_path_pkgs: &[&LockedPackageStorePath],
        span: Span,
        semaphore: &Semaphore,
    ) -> Result<(), BuildEnvError> {
        let missing: Vec<String> = base_catalog_pkgs
            .iter()
            .flat_map(|pkg| pkg.outputs.values())
            .chain(store_path_pkgs.iter().map(|pkg| &pkg.store_path))
            .filter(|p| !path_is_present(p.as_str()))
            .cloned()
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        debug!(count = missing.len(), "substituting store paths in batch");

        let mut cmd = base_command();
        cmd.arg("build")
            .arg("--no-link")
            .arg("--keep-going")
            .args(&missing);

        // Acquire one semaphore slot for the batch subprocess, consistent with
        // the per-package helpers. The slot is released before the per-package
        // fallback below so fallback calls can each acquire their own slot.
        let output = {
            let _guard = semaphore.wait();
            cmd.output().map_err(BuildEnvError::CallNixBuild)?
        };
        if output.status.success() {
            return Ok(());
        }

        debug!(
            stderr = %String::from_utf8_lossy(&output.stderr),
            "batch substitution partially failed, falling back to per-package for error reporting"
        );

        // Per-package fallback: only process packages whose paths are still
        // missing after the batch attempt. Collect all errors rather than
        // short-circuiting on the first failure so every broken package is
        // attempted and logged.
        let mut errors: Vec<BuildEnvError> = Vec::new();
        for pkg in base_catalog_pkgs {
            if pkg.outputs.values().all(|p| path_is_present(p.as_str())) {
                continue;
            }
            if let Err(e) = Self::realise_single_base_catalog_pkg(pkg, span.clone(), semaphore) {
                errors.push(e);
            }
        }
        for pkg in store_path_pkgs {
            if path_is_present(pkg.store_path.as_str()) {
                continue;
            }
            if let Err(e) = Self::realise_single_store_path(pkg, span.clone(), semaphore) {
                errors.push(e);
            }
        }

        match errors.len() {
            0 => Ok(()),
            1 => Err(errors.into_iter().next().unwrap()),
            n => {
                for e in &errors {
                    debug!(error = %e, "per-package realisation failed");
                }
                Err(BuildEnvError::Other(format!(
                    "{n} packages failed to realise; first error: {}",
                    errors[0]
                )))
            },
        }
    }

    /// Tries to substitute a single base catalog package, assuming that we have
    /// its metadata, but that the binary lives in the upstream Nix cache
    /// (e.g cache.nixos.org).
    fn realise_single_base_catalog_pkg(
        locked_pkg: &LockedPackageCatalog,
        span: Span,
        semaphore: &Semaphore,
    ) -> Result<(), BuildEnvError> {
        // Fast path: all outputs already present on disk.  Stat checks are
        // cheap and don't need concurrency limiting, so check before acquiring
        // a semaphore slot.
        if locked_pkg
            .outputs
            .values()
            .all(|p| path_is_present(p.as_str()))
        {
            return Ok(());
        }

        let _guard = semaphore.wait();

        // Attempt to download the store paths associated with the package outputs.
        let all_valid_after_build_or_substitution = {
            let span = info_span!(
                parent: span.clone(),
                "substitute catalog package",
                progress = format!("Downloading '{}'", locked_pkg.attr_path)
            );
            span.in_scope(|| Self::try_substitute_store_paths(locked_pkg.outputs.values()))?
        };

        // If all store paths are valid after substitution, we can return early.
        if all_valid_after_build_or_substitution {
            return Ok(());
        }

        // If we get here it means we need to build a package from source.
        // Delegate to the shared function so buildenv and `flox run` stay in sync.
        build_catalog_pkg_from_source(
            &locked_pkg.locked_url,
            &locked_pkg.attr_path,
            &locked_pkg.system,
            locked_pkg.unfree,
            locked_pkg.broken,
            None, // buildenv manages its own GC roots; use --no-link here
        )
    }

    /// Realise a package from a flake.
    /// [LockedPackageFlake] is a locked package from a flake installable.
    /// The package is realised by checking if the store paths are valid,
    /// and otherwise building the package to create valid store paths.
    /// Packages are built by optimistically joining the flake url and attr path,
    /// which has been previously evaluated successfully during locking,
    /// and building the package with essentially `nix build <flake-url>#<attr-path>^*`.
    /// We set `--option pure-eval true` to avoid improve reproducibility,
    /// and allow the use of the eval-cache to avoid costly re-evaluations.
    ///
    /// Note: an earlier version of this function performed a second stat check
    /// after a successful build to detect the "became valid in the meantime" case.
    /// That check is now provided by the outer [`materialise_with_retry`] loop,
    /// which re-stats all expected paths before calling `buildenv.nix`.
    #[instrument(skip_all, fields(progress = format!("Realising flake package '{}'", locked.install_id)))]
    fn realise_flake(locked: &LockedPackageFlake) -> Result<(), BuildEnvError> {
        // Fast path: all outputs already present on disk.
        if locked
            .locked_installable
            .outputs
            .values()
            .all(|p| path_is_present(p.as_str()))
        {
            return Ok(());
        }

        let mut nix_build_command = base_command();

        // naïve url construction
        let installable = {
            let locked_url = &locked.locked_installable.locked_url;
            let attr_path = &locked.locked_installable.locked_flake_attr_path;

            format!("{}#{}^*", locked_url, attr_path)
        };

        let _span = info_span!(
            "build flake package",
            progress = format!("Building '{installable}'")
        )
        .entered();

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--no-link");
        nix_build_command.arg(&installable);

        debug!(%installable, cmd=%nix_build_command.display(), "building flake package:");

        let output = nix_build_command
            .output()
            .map_err(BuildEnvError::CallNixBuild)?;

        if !output.status.success() {
            return Err(BuildEnvError::Realise2 {
                install_id: locked.install_id.clone(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Realise a package from a store path.
    /// [LockedPackageStorePath] is a locked package from a store path.
    /// The package is realised by checking if the store paths are valid,
    /// if the store path is not valid (and the store lacks the ability to reproduce it),
    /// This function will return an error.
    fn realise_single_store_path(
        locked: &LockedPackageStorePath,
        parent_span: Span,
        semaphore: &Semaphore,
    ) -> Result<(), BuildEnvError> {
        // Fast path: already present on disk.  Stat checks are cheap and
        // don't need concurrency limiting, so check before acquiring a
        // semaphore slot.
        if path_is_present(locked.store_path.as_str()) {
            return Ok(());
        }

        let _guard = semaphore.wait();

        let valid = {
            let span = info_span!(
                parent: parent_span.clone(),
                "substitute store path",
                progress = format!("Downloading '{}'", locked.store_path)
            );
            span.in_scope(|| Self::try_substitute_store_paths([&locked.store_path]))?
        };

        if !valid {
            return Err(BuildEnvError::Realise2 {
                install_id: locked.install_id.clone(),
                message: format!("'{}' is not available", locked.store_path),
            });
        }
        Ok(())
    }

    /// Fetch the given store paths via `nix build --no-link`, which will
    /// download from configured substituters or build from source if needed.
    ///
    /// Returns `true` if all paths were fetched successfully, `false`
    /// otherwise.
    fn try_substitute_store_paths(
        paths: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<bool, BuildEnvError> {
        substitute_store_paths(paths, None)
    }

    /// Build the environment by evaluating and building
    /// the `buildenv.nix` expression.
    ///
    /// The `buildenv.nix` reads the lockfile and composes
    /// an environment derivation, with outputs for the `dev` and `run` modes,
    /// as well as additional outputs for each manifest build.
    /// Note that the `buildenv.nix` expression **does not** build any of the packages!
    /// Instead it will exclusively use the store paths of the packages,
    /// that have been realised via [Self::realise_lockfile].
    /// At the moment it is required that both `buildenv.nix`
    /// and [Self::realise_lockfile], realise the same packages and outputs consistently.
    /// Future improvements will allow to pass the store paths explicitly
    /// to the `buildenv.nix` expression.
    fn call_buildenv_nix(
        &self,
        lockfile_path: &Path,
        service_config_path: Option<PathBuf>,
        out_link_prefix: Option<&Path>,
    ) -> Result<BuildEnvOutputs, BuildEnvError> {
        let mut nix_build_command = base_command();
        nix_build_command.args(["build", "--offline", "--json"]);
        if let Some(prefix) = out_link_prefix {
            nix_build_command.arg("--out-link").arg(prefix);
        } else {
            nix_build_command.arg("--no-link");
        }
        nix_build_command.arg("--file").arg(&*BUILDENV_NIX);
        nix_build_command
            .arg("--argstr")
            .arg("manifestLock")
            .arg(lockfile_path);
        if let Some(service_config_path) = &service_config_path {
            nix_build_command
                .arg("--argstr")
                .arg("serviceConfigYaml")
                .arg(service_config_path);
        }
        debug!(cmd=%nix_build_command.display(), "building environment");

        let output = nix_build_command
            .output()
            .map_err(BuildEnvError::CallNixBuild)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildEnvError::Build(stderr.to_string()));
        }
        // defined inline as an implementation detail
        #[derive(Debug, Clone, Deserialize)]
        struct BuildEnvResultRaw {
            outputs: BuildEnvOutputs,
        }
        let build_env_result: Result<[BuildEnvResultRaw; 1], _> =
            serde_json::from_slice(&output.stdout).map_err(|err| BuildEnvError::ReadOutputs {
                output: String::from_utf8_lossy(&output.stdout).to_string(),
                err,
            });

        if let Ok([build_env_result]) = build_env_result {
            let outputs = build_env_result.outputs;
            remove_build_output_symlinks(out_link_prefix, &outputs);
            return Ok(outputs);
        }

        // Preexisting store paths produced by the build may have been (partially) swept away.
        // In that case the above `nix build` only documents the _new_ outputs.
        // A second build with the same arguments will be fully substituted and contain all outputs.
        //
        // We only try this once because the window for paths to disappear between the last build
        // and this one is particularly short, incorrect output is now reliably wrong
        // and should be propagated up.
        //
        // Note: this inner retry handles only the JSON-output-parse case (partially-swept
        // preexisting paths producing incomplete output). GC-race retries at the materialisation
        // level are handled by `materialise_with_retry` — do not consolidate these two loops.
        debug!(err=%build_env_result.unwrap_err(), "failed to deserialize output, retrying once");
        debug!(cmd=%nix_build_command.display(), "building environment");

        let output = nix_build_command
            .output()
            .map_err(BuildEnvError::CallNixBuild)?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildEnvError::Build(stderr.to_string()));
        }

        let [build_env_result]: [BuildEnvResultRaw; 1] = serde_json::from_slice(&output.stdout)
            .inspect_err(|_| {
                debug!("failed to deserialize output on second try");
                remove_build_output_symlinks_by_scan(out_link_prefix);
            })
            .map_err(|err| BuildEnvError::ReadOutputs {
                output: String::from_utf8_lossy(&output.stdout).to_string(),
                err,
            })?;
        let outputs = build_env_result.outputs;
        remove_build_output_symlinks(out_link_prefix, &outputs);
        Ok(outputs)
    }
}

/// Remove the `<prefix>-build-*` symlinks that `nix build --out-link` creates
/// for manifest build outputs.
///
/// `nix build --out-link <prefix>` writes a symlink for every derivation
/// output: `<prefix>-run`, `<prefix>-dev`, and — for environments with build
/// sections — `<prefix>-build-<id>` for each manifest build output.  The
/// `build-*` symlinks clutter `.flox/run/`.  We delete them immediately after
/// the build — this also removes the GC roots for those outputs, which is
/// intentional.  The store paths are captured in
/// `outputs.manifest_build_runtimes` before deletion.
fn remove_build_output_symlinks(out_link_prefix: Option<&Path>, outputs: &BuildEnvOutputs) {
    let Some(prefix) = out_link_prefix else {
        return;
    };
    for key in outputs.manifest_build_runtimes.keys() {
        let mut link_name = prefix.as_os_str().to_owned();
        link_name.push("-");
        link_name.push(key.as_str());
        let link_path = PathBuf::from(link_name);
        if link_path.is_symlink()
            && let Err(e) = std::fs::remove_file(&link_path)
        {
            debug!(path=?link_path, error=%e, "failed to remove build output symlink from run dir");
        }
    }
}

/// Fallback version of [`remove_build_output_symlinks`] for the error path,
/// where we have a prefix but no parsed outputs to iterate over.  Scans the
/// parent directory and removes any symlink whose name starts with
/// `<prefix_filename>-build-`.
fn remove_build_output_symlinks_by_scan(out_link_prefix: Option<&Path>) {
    let Some(prefix) = out_link_prefix else {
        return;
    };
    let Some(parent) = prefix.parent() else {
        return;
    };
    let Some(prefix_name) = prefix.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let scan_prefix = format!("{prefix_name}-build-");
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name_str) = name.to_str() else {
            continue;
        };
        if name_str.starts_with(&scan_prefix)
            && entry.path().is_symlink()
            && let Err(e) = std::fs::remove_file(entry.path())
        {
            debug!(path=?entry.path(), error=%e, "failed to remove build output symlink from run dir");
        }
    }
}

/// Apply the `_FLOX_NIX_STORE_URL` store override to `cmd` if set, so that
/// Nix commands spawned outside of `base_command` (e.g. the
/// daemon-check and force-registration commands) target the same store as the
/// rest of environment construction.
fn apply_nix_store_url(cmd: &mut Command) {
    match std::env::var("_FLOX_NIX_STORE_URL").ok().as_deref() {
        None | Some("") => {
            debug!("using 'auto' store");
        },
        Some(store_url) => {
            debug!(%store_url, "overriding Nix store URL");
            cmd.args(["--option", "store", store_url]);
        },
    }
}

/// Queries the Nix daemon's store database for `paths` using `nix path-info
/// --json` and returns those that the daemon returns as `null` (i.e. paths
/// that may be present on the filesystem but are not registered in the
/// database).
fn nix_path_info_null_paths(paths: &[String]) -> Result<Vec<String>, std::io::Error> {
    if paths.is_empty() {
        return Ok(Vec::new());
    }
    let mut cmd = nix_base_command();
    apply_nix_store_url(&mut cmd);
    let output = cmd.args(["path-info", "--json"]).args(paths).output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "nix path-info exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let map: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(std::io::Error::other)?;
    Ok(map
        .into_iter()
        .filter_map(|(path, value)| value.is_null().then_some(path))
        .collect())
}

/// Returns true if `err` reports a buildenv package-output conflict from
/// `buildenv/builder.pl`. These errors are deterministic — the same lockfile
/// will always produce the same conflict — and must not be retried.
///
/// Matches on the conflict-specific resolution hint emitted at
/// `buildenv/builder.pl:245` (`Resolve by uninstalling one of the conflicting
/// packages`) rather than the broader `❌ ERROR:` sentinel installed by the
/// `$SIG{__DIE__}` handler at `buildenv/builder.pl:13`. The broader sentinel
/// covers builder.pl's filesystem `die` paths (EMFILE, ENOSPC, etc. during
/// mkpath/symlink), which are genuinely transient and must remain retryable.
///
/// If `buildenv/builder.pl`'s conflict message changes, the unit test
/// `materialise_retry_tests::deterministic_buildenv_conflict_short_circuits`
/// will break and signal a contract update is needed.
fn is_deterministic_buildenv_conflict(err: &BuildEnvError) -> bool {
    match err {
        BuildEnvError::Build(stderr) => {
            stderr.contains("Resolve by uninstalling one of the conflicting packages")
        },
        _ => false,
    }
}

/// Retry loop for materialisation and environment construction.
///
/// Calls `realise` to materialise store paths, then `missing_paths` to detect
/// any GC'd paths in the window between materialisation and the `buildenv.nix`
/// run. Missing paths after a successful `realise` indicate concurrent GC:
/// retry if attempts remain, or return
/// [`BuildEnvError::MaterialisationFailed`].
///
/// If `build_env` fails, `missing_paths` is called again to determine the
/// cause:
/// - Paths now missing: GC occurred between the pre-build stat and the
///   `buildenv.nix` run — retry if attempts remain, or return
///   [`BuildEnvError::MaterialisationFailed`].
/// - All paths still present per stat: `nix path-info --json` is run to
///   check the Nix daemon's store database.  If the daemon confirms all
///   paths are registered, retrying is still allowed because there is a
///   race window: a path can arrive on disk (passing stat()) before the
///   Nix daemon records it in its SQLite database (causing
///   `builtins.storePath` to fail), then the DB registration completes
///   before `nix path-info` runs.  Remaining retries are used to resolve
///   that window; the error is only propagated once retries are exhausted.
///   If the daemon returns `null` for any paths they are present on the
///   filesystem but not registered (e.g. a CI cache that bypassed the
///   daemon).  Those paths are force-registered with
///   `nix build --no-link --keep-going` and the attempt is retried.
///
/// Package-output conflicts detected by [`is_deterministic_buildenv_conflict`]
/// are short-circuited immediately without retrying; the same lockfile always
/// produces the same conflict.
///
/// `realise` errors always propagate immediately; a path that never appeared
/// in the store is not a GC race.
///
/// `expected_paths` returns the full list of store paths the environment
/// depends on. It is called once per attempt for diagnostic logging and
/// for the `nix path-info` store-database check on failure.
pub fn materialise_with_retry<T>(
    mut realise: impl FnMut() -> Result<(), BuildEnvError>,
    mut missing_paths: impl FnMut() -> Vec<String>,
    expected_paths: impl Fn() -> Vec<String>,
    mut build_env: impl FnMut() -> Result<T, BuildEnvError>,
) -> Result<T, BuildEnvError> {
    const MAX_RETRIES: usize = 3;
    for attempt in 1..=MAX_RETRIES {
        realise()?;
        let missing = missing_paths();
        if !missing.is_empty() {
            if attempt < MAX_RETRIES {
                debug!(
                    ?missing,
                    attempt,
                    MAX_RETRIES,
                    "store paths missing after materialisation, GC suspected — retrying"
                );
                continue;
            }
            const MAX_DISPLAY: usize = 5;
            let total = missing.len();
            let display = if total <= MAX_DISPLAY {
                missing.join("\n  ")
            } else {
                format!(
                    "{}\n  ... and {} more",
                    missing[..MAX_DISPLAY].join("\n  "),
                    total - MAX_DISPLAY
                )
            };
            debug!(paths = ?missing, "full list of missing store paths after exhausting retries");
            return Err(BuildEnvError::MaterialisationFailed {
                attempts: MAX_RETRIES,
                paths: display,
            });
        }
        let confirmed = expected_paths();
        debug!(
            attempt,
            MAX_RETRIES,
            paths = ?confirmed,
            "all store paths present per stat(), calling buildenv.nix"
        );
        match build_env() {
            Ok(outputs) => return Ok(outputs),
            Err(e) => {
                // Re-stat to distinguish a GC race from a deterministic
                // failure. A path that was present before build_env but is
                // gone afterwards means GC swept it during the run — retry.
                // If all paths are still present, check nix path-info to
                // determine whether a DB-registration race or a genuinely
                // deterministic failure (bad lockfile, spawn error) is the
                // cause.
                let missing_after = missing_paths();
                if missing_after.is_empty() {
                    // All paths still present per stat() — check whether the
                    // Nix daemon agrees. CI caches may restore store content
                    // by copying files directly to /nix/store, bypassing the
                    // daemon, leaving paths on disk but unregistered.
                    match nix_path_info_null_paths(&confirmed) {
                        Ok(null_paths) if null_paths.is_empty() => {
                            // Short-circuit deterministic package-output conflicts
                            // before spending retries. The same lockfile always
                            // produces the same conflict, so retrying cannot help.
                            if is_deterministic_buildenv_conflict(&e) {
                                debug!(
                                    error = %e,
                                    attempt,
                                    "buildenv.nix reported a deterministic \
                                    package-output conflict — not retrying"
                                );
                                return Err(e);
                            }
                            // Daemon confirms all paths are registered. This is
                            // usually a genuine deterministic failure, but there
                            // is a race: a path can arrive on disk (passing
                            // stat()) before the Nix daemon records it in its DB
                            // (causing builtins.storePath to fail), then the DB
                            // registration completes before nix path-info runs.
                            // Allow remaining retries to resolve that window;
                            // only treat it as deterministic once retries are
                            // exhausted.
                            if attempt < MAX_RETRIES {
                                debug!(
                                    error = %e,
                                    attempt,
                                    MAX_RETRIES,
                                    "buildenv.nix failed with all paths confirmed in Nix store — possible DB-registration race, retrying"
                                );
                                continue;
                            }
                            warn!(
                                error = %e,
                                attempt,
                                MAX_RETRIES,
                                "buildenv.nix failed with all paths confirmed in Nix store after all retries — treating as deterministic"
                            );
                            return Err(e);
                        },
                        Ok(null_paths) => {
                            // Some paths are on disk but not registered with
                            // the daemon. Force-register them, then retry.
                            warn!(
                                ?null_paths,
                                attempt,
                                MAX_RETRIES,
                                "buildenv.nix failed: paths present on filesystem but \
                                not registered in Nix store — force-registering and retrying"
                            );
                            let mut reg_cmd = nix_base_command();
                            apply_nix_store_url(&mut reg_cmd);
                            let reg = reg_cmd
                                .arg("build")
                                .arg("--no-link")
                                .arg("--keep-going")
                                .args(&null_paths)
                                .output()
                                .map_err(BuildEnvError::CallNixBuild)?;
                            if !reg.status.success() {
                                warn!(
                                    error = %e,
                                    status = %reg.status,
                                    stderr = %String::from_utf8_lossy(&reg.stderr),
                                    "failed to register unregistered paths — \
                                    propagating original error"
                                );
                                return Err(e);
                            }
                            if attempt < MAX_RETRIES {
                                continue;
                            }
                            // Last attempt: paths are now registered, try
                            // build_env once more without burning another
                            // loop iteration.
                            return build_env();
                        },
                        Err(path_info_err) => {
                            warn!(
                                error = %e,
                                path_info_error = %path_info_err,
                                attempt,
                                MAX_RETRIES,
                                "buildenv.nix failed and nix path-info check errored \
                                — treating as deterministic"
                            );
                            return Err(e);
                        },
                    }
                }
                if attempt < MAX_RETRIES {
                    debug!(
                        ?missing_after,
                        error = %e,
                        attempt,
                        MAX_RETRIES,
                        "store paths went missing during buildenv.nix, GC suspected — retrying"
                    );
                    continue;
                }
                const MAX_DISPLAY: usize = 5;
                let total = missing_after.len();
                let display = if total <= MAX_DISPLAY {
                    missing_after.join("\n  ")
                } else {
                    format!(
                        "{}\n  ... and {} more",
                        missing_after[..MAX_DISPLAY].join("\n  "),
                        total - MAX_DISPLAY
                    )
                };
                debug!(paths = ?missing_after, "full list of missing store paths after exhausting retries");
                return Err(BuildEnvError::MaterialisationFailed {
                    attempts: MAX_RETRIES,
                    paths: display,
                });
            },
        }
    }
    unreachable!("retry loop always returns")
}

impl<A> BuildEnv for BuildEnvNix<A>
where
    A: AuthProvider,
{
    #[instrument(skip_all, fields(progress = "Building environment"))]
    fn build(
        &self,
        client: &impl CatalogClientTrait,
        lockfile_path: &Path,
        service_config_path: Option<PathBuf>,
        out_link_prefix: Option<&Path>,
    ) -> Result<BuildEnvOutputs, BuildEnvError> {
        // Note: currently used in a single integration test to verify,
        // that the buildenv is not called a second time for remote environments,
        // that have already been built at the current revision.
        if env::var("_FLOX_TESTING_NO_BUILD").is_ok() {
            panic!("Can't build when _FLOX_TESTING_NO_BUILD is set");
        }

        let lockfile =
            Lockfile::read_from_file(&CanonicalPath::new(lockfile_path).unwrap()).unwrap();

        // Check if the lockfile is compatible with the current system.
        // Explicitly setting the `options.systems` field in the manifest,
        // has the semantics of restricting the environments to the specified systems.
        // Restricting systems can help the resolution process and avoid confusion,
        // when using the environment on unsupported systems.
        // Without this check the lockfile would succeed to build on any system,
        // but (in the general case) contain no packages,
        // because the lockfile won't contain locks of packages for the current system.
        let manifest = lockfile.migrated_manifest()?;
        let systems = &manifest.as_latest_schema().options.systems;
        if let Some(systems) = systems
            && !systems.contains(&env!("NIX_TARGET_SYSTEM").to_string())
        {
            return Err(BuildEnvError::LockfileIncompatible {
                systems: systems.clone(),
            });
        }

        // Realise the packages in the lockfile, for the current system.
        // "Realising" a package means to check if the associated store paths are valid
        // and otherwise building the package to _create_ valid store paths.
        // The following build of the `buildenv.nix` file will exclusively use
        // the now valid store paths.
        // We split the realisation of the lockfile from the build of the environment,
        // to allow finer grained control over the build process of individual packages,
        // and to avoid the performance degradation of building
        // from within an impurely evaluated nix expression.
        //
        // After each materialisation pass, stat() all paths to verify none were
        // GC'd in the narrow window between fetching and buildenv.nix running.
        // If call_buildenv_nix fails and paths are subsequently missing, GC is
        // suspected and the whole pass is retried. If all paths are still
        // present after a call_buildenv_nix failure the error is treated as
        // deterministic (spawn failure, bad lockfile, JSON parse error) and
        // propagated immediately without retrying.
        let system = env!("NIX_TARGET_SYSTEM").to_string();
        let all_env_paths: Vec<String> = lockfile
            .packages
            .iter()
            .filter(|pkg| pkg.system() == &system)
            .flat_map(|pkg| match pkg {
                LockedPackage::Catalog(p) => p.outputs.values().cloned().collect::<Vec<_>>(),
                LockedPackage::Flake(p) => p.locked_installable.outputs.values().cloned().collect(),
                LockedPackage::StorePath(p) => vec![p.store_path.clone()],
            })
            .collect();
        materialise_with_retry(
            || self.realise_lockfile(client, &lockfile, &system),
            || {
                // Check local filesystem visibility. Even when the Nix daemon
                // runs remotely (NIX_REMOTE=ssh-ng://...), Flox requires that
                // store paths are visible on the local filesystem — either via
                // an NFS-mounted /nix/store or a FUSE layer that replicates
                // paths on demand. The buildenv.nix invocation that follows
                // also produces store paths that must be locally visible, so
                // a local stat is the correct and sufficient check here.
                all_env_paths
                    .iter()
                    .filter(|path| !path_is_present(path.as_str()))
                    .cloned()
                    .collect()
            },
            || all_env_paths.clone(),
            || self.call_buildenv_nix(lockfile_path, service_config_path.clone(), out_link_prefix),
        )
    }
}

pub fn get_installed_outputs(package: &PackageToList) -> Result<Vec<String>, ManifestError> {
    let (package_name, outputs, descriptor_outputs, outputs_to_install) = match package {
        PackageToList::StorePath(_) => return Ok(vec![]),
        PackageToList::Catalog(descriptor, locked) => (
            &locked.install_id,
            &locked.outputs,
            &descriptor.outputs,
            &locked.outputs_to_install,
        ),
        PackageToList::Flake(descriptor, locked) => (
            &locked.install_id,
            &locked.locked_installable.outputs,
            &descriptor.outputs,
            &locked.locked_installable.outputs_to_install,
        ),
    };
    let all_outputs = outputs.keys().cloned().collect();

    match descriptor_outputs {
        Some(SelectedOutputs::All(_)) => Ok(all_outputs),
        Some(SelectedOutputs::Specific(selected)) => {
            let invalid_outputs: Vec<String> = selected
                .iter()
                .filter(|&o| !outputs.contains_key(o))
                .cloned()
                .collect();
            if invalid_outputs.is_empty() {
                Ok(selected.clone())
            } else {
                Err(ManifestError::InvalidOutputs(
                    invalid_outputs,
                    package_name.clone(),
                ))
            }
        },
        None => Ok(outputs_to_install.clone().unwrap_or(all_outputs)),
    }
}

/// Join all realise (download) thread handles, returning the first error encountered.
/// Thread panics are reported when no threads return an error.
fn join_realise_results(
    thread_handles: Vec<ScopedJoinHandle<'_, Result<(), BuildEnvError>>>,
) -> Result<(), BuildEnvError> {
    let mut first_error: Option<BuildEnvError> = None;
    let mut thread_panicked = false;

    for handle in thread_handles {
        match handle.join() {
            Ok(Ok(())) => {},
            Ok(Err(e)) => {
                if first_error.is_none() {
                    first_error = Some(e);
                }
            },
            Err(_) => thread_panicked = true,
        }
    }
    if let Some(err) = first_error {
        return Err(err);
    }
    if thread_panicked {
        return Err(BuildEnvError::Other(
            "internal error: download thread panicked".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod test_helpers {
    pub(super) use flox_test_utils::init_tracing;
    use tempfile::TempDir;

    use super::*;
    use crate::providers::nix_auth::NixAuth;

    pub(super) fn buildenv_instance() -> BuildEnvNix<NixAuth> {
        init_tracing();
        let auth = NixAuth::from_tempdir_and_token(TempDir::new().unwrap(), None);
        BuildEnvNix::new(auth)
    }

    /// Check if the given store paths exist on the local filesystem via
    /// `stat(2)`. Fast, optimistic check used in tests to assert presence
    /// before and after realisation.
    pub(super) fn check_store_path(
        paths: impl IntoIterator<Item = impl AsRef<str>>,
    ) -> Result<bool, BuildEnvError> {
        Ok(paths.into_iter().all(|p| path_is_present(p.as_ref())))
    }
}

#[cfg(test)]
mod realise_nixpkgs_tests {

    use flox_manifest::lockfile::test_helpers::{
        locked_package_catalog_from_mock,
        locked_published_package,
    };
    use flox_test_utils::GENERATED_DATA;
    use indoc::indoc;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::providers::nix::test_helpers::known_store_path;
    use crate::providers::nix_auth::NixAuth;

    /// When a package is not available in the store, it should be built from its derivation.
    /// This test sets a known invalid store path to trigger a rebuild of the 'hello' package.
    /// Since we're unable to provide unique store paths for each test run,
    /// this test is only indicative that we _actually_ build the package.
    #[test]
    fn nixpkgs_build_reproduce_if_invalid() {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // Note: Packages from the catalog are always possibly present already
        // especially if they are built by a previous run of the test suite.
        // hence we can't check if they are invalid before building.

        test_helpers::init_tracing();

        let result = BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        assert!(result.is_ok());

        // Note: per the above this may be incidentally true
        assert!(test_helpers::check_store_path([original_store_path]).unwrap());
    }

    /// When a package is available in the store, it should not be evaluated or built.
    /// This test sets the attribute path to a known bad value,
    /// to ensure that the build will fail if buildenv attempts to evaluate the package.
    #[test]
    fn nixpkgs_skip_eval_if_valid() {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        // build the package to ensure it is in the store
        BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect("'hello' package should build");

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();
        BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect("'hello' package should be realised without eval/build");
    }

    /// Realising a nixpkgs package should fail if the output is not valid
    /// and cannot be built.
    /// Here we are testing the case where the attribute fails to evaluate.
    /// Generally we expect packages from the catalog to be able to evaluate,
    /// iff the catalog server was able to evaluate them before.
    /// This test is a catch-all for all kinds of eval failures.
    /// Eval failures for **unfree** and **broken** packages should be prevented,
    /// which is tested in the tests below.
    #[test]
    fn nixpkgs_eval_failure() {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();

        let result = BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        let err = result.expect_err("realising nixpkgs#AAAAAASomeThingsFailToEvaluate should fail");
        assert!(matches!(err, BuildEnvError::Realise2 { .. }));
    }

    /// Ensure that we can build, or (attempt to build) a package from the catalog,
    /// that is marked as **unfree**.
    /// By default, unfree packages are included in resolution responses,
    /// unless explicitly disabled.
    /// Nixpkgs provides an _evaltime_ check for this metadata attribute,
    /// causing evaluation failures unless configured otherwise.
    /// Since we have our own control mechanism and generally want to skip evaluations
    /// if possible, we rely on [[BuildEnvNix::realise_nixpkgs]]
    /// to successfully evaluate the package and build it.
    #[test]
    fn nixpkgs_build_unfree() {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello-unfree-lock.yaml"));

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let result = BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    /// Ensure that we can build, or (attempt to build) a package from the catalog,
    /// that is marked as **broken**.
    /// Packages marked as broken may build successfully, but are not guaranteed to work.
    /// By default, the packages are not included in resolution responses,
    /// unless explicitly enabled.
    /// Nixpkgs provides an _evaltime_ check for this metadata attribute,
    /// causing evaluation failures unless configured otherwise,.
    /// Since we have our own control mechanism and generally want to skip evaluations
    /// if possible, we rely on [[BuildEnvNix::realise_nixpkgs]]
    /// to (at least) successfully evaluate the package, and attempt to build it.
    #[test]
    fn nixpkgs_build_broken() {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/tabula-lock.yaml"));

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let result = BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &locked_package,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    // FIXME: uncomment this once the catalog can tell us which stores need auth
    //        and which ones don't. Right now this returns a "missing token" error
    //        because "https://example.com" doesn't match the (short) list of stores
    //        that don't need a FloxHub token.
    // #[test]
    // fn nixpkgs_published_pkg_no_matching_response() {
    //     let locked_package = locked_published_package(None);
    //     let mut client = MockClient::new();
    //     let mut resp = StoreInfoResponse {
    //         items: std::collections::HashMap::new(),
    //     };

    //     resp.items
    //         .insert(locked_package.outputs["out"].clone(), vec![StoreInfo {
    //             url: "https://example.com".to_string(),
    //         }]);
    //     client.push_store_info_response(resp);

    //     let buildenv = buildenv_instance();
    //     let subst_resp = buildenv
    //         .try_substitute_published_pkg(&client, &locked_package)
    //         .unwrap();
    //     assert!(!subst_resp);
    // }

    #[test]
    fn nixpkgs_published_pkg_no_cache_info() {
        let (locked_package, _) = locked_published_package(None);
        let fake_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let store_locations = {
            let mut map = HashMap::new();
            map.insert(fake_store_path, vec![]);
            map
        };

        let subst_resp = BuildEnvNix::<NixAuth>::realise_single_custom_catalog_pkg(
            &locked_package,
            &store_locations,
            true,
            None,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        eprintln!("RESULT: {subst_resp:?}");
        assert!(matches!(
            subst_resp,
            Err(BuildEnvError::NoPackageStoreLocation(_))
        ));
    }

    #[test]
    fn nixpkgs_published_pkg_cache_download_success() {
        let real_storepath = known_store_path();
        let real_storepath_str = real_storepath.to_string_lossy();
        let (locked_package, _) = locked_published_package(Some(&real_storepath_str));
        let store_locations = {
            let mut map = HashMap::new();
            // This is a trick for a known storepath
            map.insert(real_storepath_str.to_string(), vec![
                // Put something invalid first, to test that we try all locations
                StoreInfo {
                    url: Some("blasphemy*".to_string()),
                    auth: None,
                    catalog: None,
                    package: None,
                    public_keys: None,
                },
                StoreInfo {
                    url: Some("daemon".to_string()),
                    auth: None,
                    catalog: None,
                    package: None,
                    public_keys: None,
                },
            ]);
            map
        };

        let dummy_netrc = Some(Path::new("/netrc"));
        let subst_resp = BuildEnvNix::<NixAuth>::realise_single_custom_catalog_pkg(
            &locked_package,
            &store_locations,
            true,
            dummy_netrc,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        eprintln!("RESULT: {subst_resp:?}");
        assert!(subst_resp.is_ok());
    }

    #[test]
    fn nixpkgs_published_pkg_cache_download_failure() {
        let (locked_package, _) = locked_published_package(None);
        let store_locations = {
            let mut map = HashMap::new();
            // This is a trick for a known storepath
            map.insert(locked_package.outputs["out"].clone(), vec![
                // Put something invalid first, to test that we try all locations
                // FIXME: uncomment this once the catalog can tell us which stores
                //        require auth and which ones don't
                // StoreInfo {
                //     url: "blasphemy*".to_string(),
                // },
                StoreInfo {
                    url: Some("daemon".to_string()),
                    auth: None,
                    catalog: None,
                    package: None,
                    public_keys: None,
                },
            ]);
            map
        };

        let result = BuildEnvNix::<NixAuth>::realise_single_custom_catalog_pkg(
            &locked_package,
            &store_locations,
            true,
            None,
            Span::current(),
            &Semaphore::new(1, 1),
        );
        let err = result.unwrap_err();
        assert!(matches!(err, BuildEnvError::BuildPublishedPackage { .. }));
        // The fallback no longer attempts a source build from locked_url (custom
        // catalog packages have no valid nixpkgs locked_url), so the base catalog
        // entry reflects a public substituter miss rather than a build error.
        assert_eq!(err.to_string(), indoc! {r#"
            Couldn't download package 'hello' from the following locations

            daemon:
              don't know how to build these paths:
                /nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid
              error: path '/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid' is required, but there is no substituter that can build it

            base catalog:
              substitution from base catalog failed"#});
    }

    /// Ensure that we can build, or (attempt to build) a package from the catalog,
    /// that is marked as **insecure**.
    /// By default, insecure packages are not included in resolution responses,
    /// unless explicitly enabled.
    /// Nixpkgs provides an _evaltime_ check for this metadata attribute,
    /// causing evaluation failures unless configured otherwise,.
    /// Since we have our own control mechanism and generally want to skip evaluations
    /// if possible, we rely on [[BuildEnvNix::realise_nixpkgs]]
    /// to (at least) successfully evaluate the package, and attempt to build it.
    #[test]
    #[ignore = "insecure packages are not yet supported by the CLI"]
    fn nixpkgs_build_insecure() {
        todo!()
    }
}

#[cfg(test)]
mod realise_flakes_tests {
    use std::fs;

    use flox_manifest::parsed::latest::PackageDescriptorFlake;
    use indoc::formatdoc;
    use tempfile::TempDir;

    use super::*;
    use crate::providers::flake_installable_locker::{InstallableLocker, InstallableLockerImpl};
    use crate::providers::nix_auth::NixAuth;

    // region: tools to configure mock flakes for testing
    struct MockedLockedPackageFlakeBuilder {
        succeed_eval: bool,
        succeed_build: bool,
        unique: bool,
    }
    impl MockedLockedPackageFlakeBuilder {
        fn new() -> Self {
            Self {
                succeed_eval: true,
                succeed_build: true,
                unique: false,
            }
        }

        fn succeed_eval(mut self, succeed: bool) -> Self {
            self.succeed_eval = succeed;
            self
        }

        fn succeed_build(mut self, succeed: bool) -> Self {
            self.succeed_build = succeed;
            self
        }

        fn unique(mut self, unique: bool) -> Self {
            self.unique = unique;
            self
        }

        fn build(self) -> MockedLockedPackageFlake {
            let tempdir = tempfile::tempdir().unwrap();

            let flake_contents = formatdoc! {r#"
                {{
                    inputs = {{ }};
                    outputs = {{ self }}: {{
                        package = let
                            builder = builtins.toFile "builder.sh" ''
                                echo "{cache_key}" > $primary
                                echo "{cache_key}"  > $secondary
                                [ "$1" = "success" ]
                                exit $?
                            '';
                        in
                        builtins.derivation {{
                            name = "{result}";
                            system = "{system}";
                            outputs = [ "primary" "secondary" ];
                            builder = "/bin/sh";
                            args = [ "${{builder}}" "{result}" ];
                        }};
                    }};
                }}
                "#,
                cache_key = if self.unique { tempdir.path().display().to_string() } else { "static".to_string() },
                result = match self.succeed_build {
                    true => "success",
                    false => "fail",
                },
                system = env!("NIX_TARGET_SYSTEM"),
            };
            fs::write(tempdir.path().join("flake.nix"), flake_contents).unwrap();
            let mut locked_installable = InstallableLockerImpl::default()
                .lock_flake_installable(env!("NIX_TARGET_SYSTEM"), &PackageDescriptorFlake {
                    flake: format!(
                        "path:{}#package",
                        tempdir.path().canonicalize().unwrap().display()
                    ),
                    systems: None,
                    priority: None,
                    outputs: None,
                })
                .unwrap();

            // We cause an eval failure by not providing a valid flake.
            // The locked_url must be overwritten,
            // as nix will otherwise use a cached version of the original flake.
            if !self.succeed_eval {
                fs::write(
                    tempdir.path().join("flake.nix"),
                    r#"{ outputs = throw "should not eval""#,
                )
                .unwrap();
                locked_installable.locked_url =
                    format!("path:{}", tempdir.path().canonicalize().unwrap().display());
            }

            let locked_package = LockedPackageFlake {
                install_id: "mock".to_string(),
                locked_installable,
            };

            MockedLockedPackageFlake {
                _tempdir: tempdir,
                locked_package,
            }
        }
    }

    #[derive(Debug, derive_more::Deref, derive_more::DerefMut)]
    struct MockedLockedPackageFlake {
        _tempdir: TempDir,
        #[deref]
        #[deref_mut]
        locked_package: LockedPackageFlake,
    }

    impl MockedLockedPackageFlake {
        fn builder() -> MockedLockedPackageFlakeBuilder {
            MockedLockedPackageFlakeBuilder::new()
        }
    }

    // endregion

    /// Flake outputs are built successfully if invalid.
    #[test]
    fn flake_build_success() {
        let locked_package = MockedLockedPackageFlake::builder().unique(true).build();

        assert!(
            !test_helpers::check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be invalid before building"
        );

        let result = BuildEnvNix::<NixAuth>::realise_flake(&locked_package);
        assert!(
            result.is_ok(),
            "failed to build flake: {}",
            result.unwrap_err()
        );
        assert!(
            test_helpers::check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap()
        );
    }

    /// Realising a flake should fail if the output is not valid and cannot be built.
    #[test]
    fn flake_build_failure() {
        let locked_package = MockedLockedPackageFlake::builder()
            .succeed_build(false)
            .unique(true)
            .build();
        let result = BuildEnvNix::<NixAuth>::realise_flake(&locked_package);
        let err = result.expect_err("realising flake should fail");
        assert!(matches!(err, BuildEnvError::Realise2 { .. }));
    }

    /// Realising a flake should fail if the output is not valid and the source cannot be evaluated.
    #[test]
    fn flake_eval_failure() {
        let locked_package = MockedLockedPackageFlake::builder()
            .succeed_eval(false)
            .unique(true)
            .build();

        assert!(
            !test_helpers::check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be invalid before building"
        );

        let result = BuildEnvNix::<NixAuth>::realise_flake(&locked_package);
        let err = result.expect_err("realising flake should fail");
        assert!(matches!(err, BuildEnvError::Realise2 { .. }));
    }

    /// Evaluation (and build) are skipped if the store path is already valid.
    #[test]
    fn flake_no_build_if_cached() {
        let mut locked_package = MockedLockedPackageFlake::builder()
            .succeed_eval(false)
            .build();

        for locked_path in locked_package.locked_installable.outputs.values_mut() {
            *locked_path = env!("GIT_PKG").to_string();
        }

        assert!(
            test_helpers::check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be valid before building"
        );

        let result = BuildEnvNix::<NixAuth>::realise_flake(&locked_package);
        assert!(result.is_ok(), "failed to skip building flake");
    }
}

#[cfg(test)]
mod realise_store_path_tests {
    use flox_manifest::parsed::common::DEFAULT_PRIORITY;

    use super::*;
    use crate::providers::nix_auth::NixAuth;

    fn mock_store_path(valid: bool) -> LockedPackageStorePath {
        LockedPackageStorePath {
            install_id: "mock".to_string(),
            store_path: if valid {
                env!("GIT_PKG").to_string()
            } else {
                "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string()
            },
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            priority: DEFAULT_PRIORITY,
        }
    }

    #[test]
    fn store_path_build_success_if_valid() {
        let locked = mock_store_path(true);

        // show that the store path is valid
        assert!(test_helpers::check_store_path([&locked.store_path]).unwrap());
        let span = info_span!("dummy");

        BuildEnvNix::<NixAuth>::realise_single_store_path(&locked, span, &Semaphore::new(1, 1))
            .expect("an existing store path should realise");
    }

    #[test]
    fn store_path_build_failure_if_invalid() {
        let locked = mock_store_path(false);

        // show that the store path is invalid
        assert!(!test_helpers::check_store_path([&locked.store_path]).unwrap());
        let span = info_span!("dummy");

        let result =
            BuildEnvNix::<NixAuth>::realise_single_store_path(&locked, span, &Semaphore::new(1, 1))
                .expect_err("invalid store path should fail to realise");
        assert!(matches!(result, BuildEnvError::Realise2 { .. }));
    }
}

#[cfg(test)]
mod realise_batch_tests {
    use flox_manifest::lockfile::test_helpers::locked_package_catalog_from_mock;
    use flox_manifest::parsed::common::DEFAULT_PRIORITY;
    use flox_test_utils::GENERATED_DATA;

    use super::*;
    use crate::providers::nix_auth::NixAuth;

    fn invalid_catalog_pkg(install_id: &str, path_suffix: &str) -> LockedPackageCatalog {
        let (mut pkg, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        pkg.install_id = install_id.to_string();
        *pkg.outputs.get_mut("out").unwrap() =
            format!("/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid-{path_suffix}");
        pkg.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();
        pkg
    }

    /// Fast path: all output paths already on disk → returns Ok without calling
    /// nix. The attr_path is poisoned so any attempted nix call would fail,
    /// proving the function returned before reaching the batch build.
    #[test]
    fn batch_succeeds_when_all_paths_already_registered() {
        let (mut catalog_pkg, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        BuildEnvNix::<NixAuth>::realise_single_base_catalog_pkg(
            &catalog_pkg,
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect("hello should be realisable before this test");

        catalog_pkg.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();

        let store_pkg = LockedPackageStorePath {
            install_id: "git".to_string(),
            store_path: env!("GIT_PKG").to_string(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            priority: DEFAULT_PRIORITY,
        };

        BuildEnvNix::<NixAuth>::realise_base_and_store_batch(
            &[&catalog_pkg],
            &[&store_pkg],
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect("all paths registered in Nix store — batch should be a no-op");
    }

    /// Batch substitution: paths may be missing but the package is valid and
    /// substitutable. Returns Ok whether via fast path (already cached) or batch.
    #[test]
    fn batch_succeeds_for_valid_packages() {
        let (catalog_pkg, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        let store_pkg = LockedPackageStorePath {
            install_id: "git".to_string(),
            store_path: env!("GIT_PKG").to_string(),
            system: env!("NIX_TARGET_SYSTEM").to_string(),
            priority: DEFAULT_PRIORITY,
        };

        BuildEnvNix::<NixAuth>::realise_base_and_store_batch(
            &[&catalog_pkg],
            &[&store_pkg],
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect("valid packages should realise via batch substitution or fast path");
    }

    /// Single failing package: batch fails → per-package fallback → Realise2.
    #[test]
    fn batch_single_failure_returns_realise2() {
        let pkg = invalid_catalog_pkg("hello", "a");

        let err = BuildEnvNix::<NixAuth>::realise_base_and_store_batch(
            &[&pkg],
            &[],
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect_err("unrealisable package should return an error");

        assert!(
            matches!(err, BuildEnvError::Realise2 { .. }),
            "expected Realise2, got: {err}"
        );
    }

    /// Two failing packages: fallback collects both errors and returns Other with count.
    /// This exercises the I4 fix: all per-package errors are collected rather than
    /// short-circuiting on the first failure.
    #[test]
    fn batch_multiple_failures_returns_error_with_count() {
        let pkg1 = invalid_catalog_pkg("hello1", "a");
        let pkg2 = invalid_catalog_pkg("hello2", "b");

        let err = BuildEnvNix::<NixAuth>::realise_base_and_store_batch(
            &[&pkg1, &pkg2],
            &[],
            Span::current(),
            &Semaphore::new(1, 1),
        )
        .expect_err("two unrealisable packages should return an error");

        match &err {
            BuildEnvError::Other(msg) => {
                assert!(
                    msg.contains("2 packages"),
                    "expected '2 packages' in error message, got: {msg}"
                );
            },
            _ => panic!("expected BuildEnvError::Other, got: {err}"),
        }
    }
}

#[cfg(test)]
mod buildenv_tests {
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;

    use flox_test_utils::{GENERATED_DATA, MANUALLY_GENERATED};
    use test_helpers::buildenv_instance;

    use super::*;
    use crate::providers::catalog::MockClient;

    trait PathExt {
        fn is_executable_file(&self) -> bool;
    }

    impl PathExt for Path {
        fn is_executable_file(&self) -> bool {
            self.is_file() && self.metadata().unwrap().permissions().mode() & 0o111 != 0
        }
    }

    static BUILDENV_RESULT_SIMPLE_PACKAGE: LazyLock<BuildEnvOutputs> = LazyLock::new(|| {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/hello/manifest.lock");
        let client = MockClient::new();
        buildenv.build(&client, &lockfile_path, None, None).unwrap()
    });

    #[test]
    fn build_contains_binaries() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.run;
        assert!(runtime.join("bin/hello").exists());
        assert!(runtime.join("bin/hello").is_executable_file());

        let develop = result.dev.as_ref();
        assert!(develop.join("bin/hello").exists());
        assert!(develop.join("bin/hello").is_executable_file());
    }

    #[test]
    fn build_contains_activate_files() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.run;
        assert!(runtime.join("activate").exists());
        assert!(runtime.join("activate.d/zsh").exists());
        assert!(runtime.join("etc/profile.d").is_dir());

        let develop = &result.dev;
        assert!(develop.join("activate").exists());
        assert!(develop.join("activate.d/zsh").exists());
        assert!(develop.join("etc/profile.d").is_dir());
    }

    #[test]
    fn build_contains_lockfile() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.run;
        assert!(runtime.join("manifest.lock").exists());

        let develop = &result.dev;
        assert!(develop.join("manifest.lock").exists());
    }
    #[test]
    fn build_contains_build_script_and_output() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-noop/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        let runtime = result.run.as_ref();
        let develop = result.dev.as_ref();
        let build_hello = result.manifest_build_runtimes.get("build-hello").unwrap();

        assert!(runtime.join("package-builds.d/hello").exists());
        assert!(develop.join("package-builds.d/hello").exists());
        assert!(build_hello.join("package-builds.d/hello").exists());
    }

    #[test]
    fn build_contains_on_activate_script() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/kitchen_sink/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        let runtime = &result.run;
        assert!(runtime.join("activate.d/hook-on-activate").exists());

        let develop = &result.dev;
        assert!(develop.join("activate.d/hook-on-activate").exists());
    }

    #[test]
    fn build_contains_profile_scripts() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/kitchen_sink/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        for output in [&result.run, &result.dev] {
            for shell in ["common", "zsh", "fish", "bash", "tcsh"] {
                assert!(
                    output.join(format!("activate.d/profile-{shell}")).exists(),
                    "profile script 'activate.d/profile-{shell}' did not exist in output {}",
                    output.display()
                );
            }
        }
    }

    #[test]
    fn verify_contents_of_requisites_txt() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;

        let runtime = result.run.as_ref();
        let develop = result.dev.as_ref();

        for out_path in [runtime, develop] {
            let requisites_path = out_path.join("requisites.txt");
            assert!(requisites_path.exists());

            let requisites: HashSet<String> = std::fs::read_to_string(&requisites_path)
                .unwrap()
                .lines()
                .map(String::from)
                .collect();

            let output = Command::new("nix-store")
                .arg("-qR")
                .arg(out_path)
                .output()
                .expect("failed to execute process");

            assert!(output.status.success());

            let store_paths: HashSet<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(String::from)
                .collect();

            assert_eq!(requisites, store_paths);
        }
    }

    #[test]
    fn detects_conflicting_packages() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/vim-vim-full-conflict.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None);
        let err = result.expect_err("conflicting packages should fail to build");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        let expected = "> ❌ ERROR: 'vim' conflicts with 'vim-full'.";

        assert!(
            output.contains(expected),
            "expected output to contain a conflict message:\n\
            actual: {output}\n\
            expected: {expected}"
        );
    }

    #[test]
    fn resolves_conflicting_packages_with_priority() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/vim-vim-full-conflict-resolved.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None);
        assert!(
            result.is_ok(),
            "conflicting packages should be resolved by priority: {}",
            result.unwrap_err()
        );
    }

    /// Older versions of Flox rendered unspecified script fields as `null` in
    /// the lockfile, which we should still support building and re-locking.
    #[test]
    fn null_script_fields() {
        let buildenv = buildenv_instance();
        let lockfile_path = MANUALLY_GENERATED.join("buildenv/lockfiles/null_fields/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None);
        assert!(
            result.is_ok(),
            "environment should render successfully: {}",
            result.unwrap_err()
        );
    }

    /// Single quotes in variables should be escaped.
    /// Similarly accidentally escaped single quotes like
    ///
    /// ```text
    /// [vars]
    /// singlequoteescaped = "\\'baz"
    /// ```
    /// should be escaped and printed as   `\'baz` (literally)
    #[test]
    fn environment_escapes_variables() {
        let buildenv = buildenv_instance();
        let lockfile_path = MANUALLY_GENERATED.join("buildenv/lockfiles/vars_escape/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        let runtime = result.run.as_ref();
        let develop = result.dev.as_ref();

        for envrc_path in [
            runtime.join("activate.d/envrc"),
            develop.join("activate.d/envrc"),
        ] {
            assert!(envrc_path.exists());
            let content = std::fs::read_to_string(&envrc_path).unwrap();
            assert!(content.contains(r#"export singlequotes="'bar'""#));
            assert!(content.contains(r#"export singlequoteescaped="\'baz""#));
        }
    }

    #[test]
    fn verify_build_closure_contains_only_toplevel_packages() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-all-toplevel.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        let runtime = result.run.as_ref();
        let develop = result.dev.as_ref();
        let build_myhello = result.manifest_build_runtimes.get("build-myhello").unwrap();

        assert!(runtime.join("bin/hello").is_executable_file());
        assert!(develop.join("bin/hello").is_executable_file());
        assert!(build_myhello.join("bin/hello").is_executable_file());

        assert!(runtime.join("bin/coreutils").is_executable_file());
        assert!(develop.join("bin/coreutils").is_executable_file());
        assert!(build_myhello.join("bin/coreutils").is_executable_file());

        assert!(runtime.join("bin/vim").is_executable_file());
        assert!(develop.join("bin/vim").is_executable_file());
        assert!(!build_myhello.join("bin/vim").exists());
    }

    #[test]
    fn verify_build_closure_contains_only_hello_with_runtime_packages_attribute() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-only-hello.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None).unwrap();

        let runtime = result.run.as_ref();
        let develop = result.dev.as_ref();
        let build_myhello = result.manifest_build_runtimes.get("build-myhello").unwrap();

        assert!(runtime.join("bin/hello").is_executable_file());
        assert!(develop.join("bin/hello").is_executable_file());
        assert!(build_myhello.join("bin/hello").is_executable_file());

        assert!(runtime.join("bin/coreutils").is_executable_file());
        assert!(develop.join("bin/coreutils").is_executable_file());
        assert!(!build_myhello.join("bin/coreutils").exists());

        assert!(runtime.join("bin/vim").is_executable_file());
        assert!(develop.join("bin/vim").is_executable_file());
        assert!(!build_myhello.join("bin/vim").exists());
    }

    #[test]
    fn verify_build_closure_can_only_select_toplevel_packages_from_runtime_packages_attribute() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-not-toplevel.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None);
        let err = result.expect_err("build should fail if non-toplevel packages are selected");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        let expected = "❌ ERROR: package 'vim' is not in 'toplevel' pkg-group";

        assert!(
            output.contains(expected),
            "expected output to contain an error message\n\
            actual: {output}\n\
            expected: {expected}"
        );
    }

    #[test]
    fn verify_build_closure_cannot_select_nonexistent_packages_in_runtime_packages_attribute() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-not-found.yaml");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None, None);
        let err = result.expect_err("build should fail if nonexistent packages are selected");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        let expected = "❌ ERROR: package 'goodbye' not found in '[install]' section of manifest";

        assert!(
            output.contains(expected),
            "expected output to contain an error message\n\
            actual: {output}\n\
            expected: {expected}"
        );
    }

    #[test]
    fn default_outputs_include_man() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with no outputs specified (should use outputs_to_install)
        let lockfile_path = GENERATED_DATA.join("envs/bash_v1_10_0_default/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.run.as_ref();
        let develop = outputs.dev.as_ref();

        // For the `bash` package the full list of outputs is:
        //
        // - out (bin/bash)
        // - man (share/man)
        // - info (share/info)
        // - doc (share/doc)
        // - dev (include)
        //
        // and `outputs_to_install` is:
        //
        // - out
        // - man
        //
        // So we expect "out" and "man" to be included by default
        assert!(
            runtime.join("bin/bash").exists(),
            "bin/bash should exist in runtime closure"
        );
        assert!(
            develop.join("bin/bash").exists(),
            "bin/bash should exist in develop closure"
        );
        assert!(
            runtime.join("share/man").exists(),
            "share/man should exist in runtime closure"
        );
        assert!(
            develop.join("share/man").exists(),
            "share/man should exist in develop closure"
        );
        // Directories from other outputs shouldn't exist
        assert!(
            !runtime.join("share/info").exists(),
            "share/info should not exist in runtime environment with default outputs"
        );
        assert!(
            !develop.join("share/info").exists(),
            "share/info should not exist in develop environment with default outputs"
        );
        assert!(
            !runtime.join("share/doc").exists(),
            "share/doc should not exist in runtime environment with default outputs"
        );
        assert!(
            !develop.join("share/doc").exists(),
            "share/doc should not exist in develop environment with default outputs"
        );
        assert!(
            !runtime.join("include").exists(),
            "include should not exist in runtime environment with default outputs"
        );
        assert!(
            !develop.join("include").exists(),
            "include should not exist in develop environment with default outputs"
        );
    }

    #[test]
    fn all_outputs_includes_info() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with outputs = "all"
        let lockfile_path = GENERATED_DATA.join("envs/bash_v1_10_0_all/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.run.as_ref();
        let develop = outputs.dev.as_ref();

        // For the `bash` package the full list of outputs is:
        //
        // - out (bin/bash)
        // - man (share/man)
        // - info (share/info)
        // - doc (share/doc)
        // - dev (include)
        assert!(
            runtime.join("bin/bash").exists(),
            "bin/bash should exist in runtime environment with outputs='all'"
        );
        assert!(
            develop.join("bin/bash").exists(),
            "bin/bash should exist in develop environment with outputs='all'"
        );
        assert!(
            runtime.join("share/man").exists(),
            "share/man should exist in runtime environment with outputs='all'"
        );
        assert!(
            develop.join("share/man").exists(),
            "share/man should exist in develop environment with outputs='all'"
        );
        assert!(
            runtime.join("share/info").exists(),
            "share/info should exist in runtime environment with outputs='all'"
        );
        assert!(
            develop.join("share/info").exists(),
            "share/info should exist in develop environment with outputs='all'"
        );
        assert!(
            runtime.join("share/doc").exists(),
            "share/doc should exist in runtime environment with outputs='all'"
        );
        assert!(
            develop.join("share/doc").exists(),
            "share/doc should exist in develop environment with outputs='all'"
        );
        assert!(
            runtime.join("include").exists(),
            "include should exist in runtime environment with outputs='all'"
        );
        assert!(
            develop.join("include").exists(),
            "include should exist in develop environment with outputs='all'"
        );
    }

    #[test]
    fn outputs_out_only_excludes_others() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with outputs = ["out"]
        let lockfile_path = GENERATED_DATA.join("envs/bash_v1_10_0_out/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.run.as_ref();
        let develop = outputs.dev.as_ref();

        // For the `bash` package the full list of outputs is:
        //
        // - out (bin/bash)
        // - man (share/man)
        // - info (share/info)
        // - doc (share/doc)
        // - dev (include)
        //
        // Since we're only including "out", none of the other directories
        // should exist
        assert!(
            runtime.join("bin/bash").exists(),
            "bin/bash should exist in runtime environment with outputs=['out']"
        );
        assert!(
            develop.join("bin/bash").exists(),
            "bin/bash should exist in develop environment with outputs=['out']"
        );
        assert!(
            !runtime.join("share/man").exists(),
            "share/man should not exist in runtime environment with outputs=['out']"
        );
        assert!(
            !develop.join("share/man").exists(),
            "share/man should not exist in develop environment with outputs=['out']"
        );
        assert!(
            !runtime.join("share/info").exists(),
            "share/info should not exist in runtime environment with outputs=['out']"
        );
        assert!(
            !develop.join("share/info").exists(),
            "share/info should not exist in develop environment with outputs=['out']"
        );
        assert!(
            !runtime.join("share/doc").exists(),
            "share/doc should not exist in runtime environment with outputs=['out']"
        );
        assert!(
            !develop.join("share/doc").exists(),
            "share/doc should not exist in develop environment with outputs=['out']"
        );
        assert!(
            !runtime.join("include").exists(),
            "include should not exist in runtime environment with outputs=['out']"
        );
        assert!(
            !develop.join("include").exists(),
            "include should not exist in develop environment with outputs=['out']"
        );
    }
}

#[cfg(test)]
mod join_realise_results_tests {
    use super::*;

    #[test]
    fn all_succeed() {
        let result = std::thread::scope(|s| {
            let handles = vec![s.spawn(|| Ok::<(), BuildEnvError>(())), s.spawn(|| Ok(()))];
            join_realise_results(handles)
        });
        assert!(result.is_ok());
    }

    #[test]
    fn single_error_is_returned() {
        let result = std::thread::scope(|s| {
            let handles = vec![
                s.spawn(|| Ok::<(), BuildEnvError>(())),
                s.spawn(|| Err(BuildEnvError::UntrustedPackage("pkg".into()))),
                s.spawn(|| Ok::<(), BuildEnvError>(())),
            ];
            join_realise_results(handles)
        });
        assert!(
            matches!(result, Err(BuildEnvError::UntrustedPackage(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn first_error_in_join_order_wins() {
        let result = std::thread::scope(|s| {
            let handles = vec![
                s.spawn(|| Err::<(), BuildEnvError>(BuildEnvError::NixCopyError("first".into()))),
                s.spawn(|| Err(BuildEnvError::UntrustedPackage("second".into()))),
                s.spawn(|| Err(BuildEnvError::Other("third".into()))),
            ];
            join_realise_results(handles)
        });
        assert!(
            matches!(result, Err(BuildEnvError::NixCopyError(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn errors_take_precedence_over_panics() {
        let result = std::thread::scope(|s| {
            let handles = vec![
                s.spawn(|| -> Result<(), BuildEnvError> { panic!("boom") }),
                s.spawn(|| Err(BuildEnvError::UntrustedPackage("pkg".into()))),
            ];
            join_realise_results(handles)
        });
        assert!(
            matches!(result, Err(BuildEnvError::UntrustedPackage(_))),
            "got {result:?}"
        );
    }

    #[test]
    fn panic_reported_when_no_errors() {
        let result = std::thread::scope(|s| {
            let handles = vec![
                s.spawn(|| Ok::<(), BuildEnvError>(())),
                s.spawn(|| -> Result<(), BuildEnvError> { panic!("boom") }),
            ];
            join_realise_results(handles)
        });
        assert!(
            matches!(result, Err(BuildEnvError::Other(_))),
            "got {result:?}"
        );
    }
}

#[cfg(test)]
mod materialise_retry_tests {
    use std::cell::Cell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    use test_helpers::init_tracing;

    use super::*;

    fn fake_outputs() -> BuildEnvOutputs {
        BuildEnvOutputs {
            dev: BuiltStorePath(PathBuf::from("/nix/store/fake-develop")),
            run: BuiltStorePath(PathBuf::from("/nix/store/fake-runtime")),
            manifest_build_runtimes: HashMap::new(),
        }
    }

    #[test]
    fn succeeds_on_first_attempt_when_paths_present() {
        init_tracing();
        let result = materialise_with_retry(|| Ok(()), Vec::new, Vec::new, || Ok(fake_outputs()));
        assert_eq!(result.unwrap(), fake_outputs());
    }

    // --- pre-build GC detection (missing paths before build_env) ---

    #[test]
    fn retries_when_paths_missing_before_build_env_then_succeeds() {
        init_tracing();
        // missing_paths call count:
        //   attempt 1: pre-build → missing (GC detected, retry)
        //   attempt 2: pre-build → present
        let call = Cell::new(0usize);
        let realise_calls = Cell::new(0usize);
        let result = materialise_with_retry(
            || {
                realise_calls.set(realise_calls.get() + 1);
                Ok(())
            },
            || {
                call.set(call.get() + 1);
                if call.get() == 1 {
                    vec!["/nix/store/aaaa-missing".to_string()]
                } else {
                    vec![]
                }
            },
            Vec::new,
            || Ok(fake_outputs()),
        );
        assert_eq!(result.unwrap(), fake_outputs());
        assert_eq!(
            realise_calls.get(),
            2,
            "realise must be called on each attempt"
        );
    }

    #[test]
    fn materialisation_failed_when_paths_always_missing_before_build_env() {
        init_tracing();
        let missing = vec![
            "/nix/store/aaaa-missing".to_string(),
            "/nix/store/bbbb-missing".to_string(),
        ];
        let result = materialise_with_retry(
            || Ok(()),
            || missing.clone(),
            Vec::new,
            || Ok(fake_outputs()),
        );
        match result.unwrap_err() {
            BuildEnvError::MaterialisationFailed { attempts, paths } => {
                assert_eq!(attempts, 3);
                assert_eq!(paths, "/nix/store/aaaa-missing\n  /nix/store/bbbb-missing");
            },
            e => panic!("expected MaterialisationFailed, got {e:?}"),
        }
    }

    // --- post-build GC detection (missing paths discovered after build_env fails) ---

    #[test]
    fn retries_when_gc_detected_after_build_env_failure() {
        init_tracing();
        // Sequence:
        //   attempt 1: pre-build → present; build_env → Err; post-build → missing (GC, retry)
        //   attempt 2: pre-build → present; build_env → Ok
        let missing_call = Cell::new(0usize);
        let realise_calls = Cell::new(0usize);
        let build_calls = Cell::new(0usize);
        let result = materialise_with_retry(
            || {
                realise_calls.set(realise_calls.get() + 1);
                Ok(())
            },
            || {
                missing_call.set(missing_call.get() + 1);
                match missing_call.get() {
                    1 => vec![],                                      // pre-build attempt 1: present
                    2 => vec!["/nix/store/aaaa-missing".to_string()], // post-build attempt 1: GC!
                    _ => vec![], // pre-build attempt 2: present
                }
            },
            Vec::new,
            || {
                build_calls.set(build_calls.get() + 1);
                if build_calls.get() == 1 {
                    Err(BuildEnvError::Build("nix build failed".to_string()))
                } else {
                    Ok(fake_outputs())
                }
            },
        );
        assert_eq!(result.unwrap(), fake_outputs());
        assert_eq!(
            realise_calls.get(),
            2,
            "realise must be called on each attempt"
        );
        assert_eq!(build_calls.get(), 2);
    }

    #[test]
    fn materialisation_failed_when_gc_detected_after_build_env_on_final_attempt() {
        init_tracing();
        // Paths always disappear after build_env fails — exhausts all retries.
        let missing_call = Cell::new(0usize);
        let result: Result<BuildEnvOutputs, _> = materialise_with_retry(
            || Ok(()),
            || {
                missing_call.set(missing_call.get() + 1);
                // pre-build checks always show paths present; post-build always missing
                if missing_call.get().is_multiple_of(2) {
                    vec!["/nix/store/aaaa-missing".to_string()]
                } else {
                    vec![]
                }
            },
            Vec::new,
            || Err(BuildEnvError::Build("nix build failed".to_string())),
        );
        match result.unwrap_err() {
            BuildEnvError::MaterialisationFailed { attempts, paths } => {
                assert_eq!(attempts, 3);
                assert_eq!(paths, "/nix/store/aaaa-missing");
            },
            e => panic!("expected MaterialisationFailed, got {e:?}"),
        }
    }

    #[test]
    fn build_env_error_exhausts_retries_when_paths_confirmed_present() {
        init_tracing();
        // build_env always fails and re-stat always shows nothing missing.
        // nix path-info (called with an empty expected_paths list) returns no
        // null paths, so we cannot distinguish a genuine deterministic failure
        // from the DB-registration race window.  The loop must use all retries
        // before propagating the error.
        let realise_calls = Cell::new(0usize);
        let build_calls = Cell::new(0usize);
        let result: Result<BuildEnvOutputs, _> = materialise_with_retry(
            || {
                realise_calls.set(realise_calls.get() + 1);
                Ok(())
            },
            Vec::new, // paths always present
            Vec::new,
            || {
                build_calls.set(build_calls.get() + 1);
                Err(BuildEnvError::Build("deterministic failure".to_string()))
            },
        );
        match result.unwrap_err() {
            BuildEnvError::Build(msg) => assert_eq!(msg, "deterministic failure"),
            e => panic!("expected Build, got {e:?}"),
        }
        assert_eq!(
            realise_calls.get(),
            3,
            "must exhaust all retries before propagating"
        );
        assert_eq!(build_calls.get(), 3, "build_env must be retried");
    }

    // --- deterministic conflict short-circuit ---

    #[test]
    fn deterministic_buildenv_conflict_short_circuits() {
        init_tracing();
        // build_env always returns a conflict error containing the
        // builder.pl resolution hint. realise and missing_paths are
        // wired to never block progress so the conflict is the only
        // reason for failure.
        let realise_calls = Cell::new(0usize);
        let build_calls = Cell::new(0usize);
        let conflict_stderr = "environment> ❌ ERROR: 'vim' conflicts with \
                               'vim-full'. Both packages provide the file \
                               'bin/ex'\nenvironment> \nenvironment> Resolve \
                               by uninstalling one of the conflicting \
                               packages or setting the priority of the \
                               preferred package to a value lower than '5'"
            .to_string();
        let result: Result<(), _> = materialise_with_retry(
            || {
                realise_calls.set(realise_calls.get() + 1);
                Ok(())
            },
            Vec::new, // paths always present
            Vec::new,
            || {
                build_calls.set(build_calls.get() + 1);
                Err(BuildEnvError::Build(conflict_stderr.clone()))
            },
        );
        match result.unwrap_err() {
            BuildEnvError::Build(msg) => assert_eq!(msg, conflict_stderr),
            e => panic!("expected Build, got {e:?}"),
        }
        assert_eq!(
            build_calls.get(),
            1,
            "must not retry a deterministic package-output conflict"
        );
        assert_eq!(
            realise_calls.get(),
            1,
            "realise called once before short-circuit"
        );
    }

    // --- realise errors ---

    #[test]
    fn realise_error_propagates_immediately() {
        init_tracing();
        let build_called = Cell::new(false);
        let result = materialise_with_retry(
            || Err(BuildEnvError::Build("realise failed".to_string())),
            Vec::new,
            Vec::new,
            || {
                build_called.set(true);
                Ok(fake_outputs())
            },
        );
        assert!(matches!(result.unwrap_err(), BuildEnvError::Build(_)));
        assert!(
            !build_called.get(),
            "build_env must not be called if realise fails"
        );
    }
}
