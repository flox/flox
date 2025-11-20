use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::hash::Hash;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, LazyLock, Mutex};

use flox_core::canonical_path::CanonicalPath;
use pollster::FutureExt as _;
use serde::{Deserialize, Serialize};
use tempfile::TempPath;
use thiserror::Error;
use tracing::{Span, debug, info_span, instrument, trace};

use super::auth::{AuthError, AuthProvider};
use super::catalog::ClientTrait;
use super::nix::{self, nix_base_command};
use crate::data::System;
use crate::models::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
    Lockfile,
};
use crate::models::manifest::typed::{ActivateMode, ManifestPackageDescriptor};
use crate::models::nix_plugins::NIX_PLUGINS;
use crate::providers::auth::{catalog_auth_to_envs, store_needs_auth};
use crate::providers::catalog::{CatalogClientError, StoreInfo};
use crate::utils::CommandExt;

static BUILDENV_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILDENV_NIX")
        .unwrap_or_else(|_| env!("FLOX_BUILDENV_NIX").to_string())
        .into()
});

/// Profix of locked_url of catalog packages that are from the nixpkgs base-catalog.
/// This url was meant to serve as a flake reference to the Flox hosted mirror of nixpkgs,
/// but is both ill formatted and does not provide the necessary overrides
/// to allow evaluating packages without common evaluation checks, such as unfree and broken.
const NIXPKGS_CATALOG_URL_PREFIX: &str = "https://github.com/flox/nixpkgs?rev=";

/// The base flake reference invoking the `flox-nixpkgs` fetcher.
/// This is a bridge to the Flox hosted mirror of nixpkgs flake,
/// which enables building packages without common evaluation checks,
/// such as unfree and broken.
const FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE: &str = "flox-nixpkgs:v0/flox";

#[derive(Debug, Error)]
pub enum BuildEnvError {
    #[error("Failed to realise '{install_id}':\n{message}")]
    Realise2 { install_id: String, message: String },

    /// An error that occurred while composing the environment.
    /// I.e. `nix build` returned with a non-zero exit code.
    /// The error message is the stderr of the `nix build` command.
    // TODO: this requires to capture the stderr of the `nix build` command
    // or essentially "tee" it if we also want to forward the logs to the user.
    // At the moment the "interesting" logs
    // are emitted by the `realise` portion of the build.
    // So in the interest of initial simplicity
    // we can defer forwarding the nix build logs and capture output with [Command::output].
    #[error("Failed to construct environment: {0}")]
    Build(String),

    #[error(
        "Lockfile is not compatible with the current system\n\
        Supported systems: {0}", systems.join(", "))]
    LockfileIncompatible { systems: Vec<String> },

    /// An error that occurred while linking a store path.
    #[error("Failed to link environment: {0}")]
    Link(String),

    /// An error that occurred while calling the client
    #[error("Unexpected error calling the catalog client")]
    CatalogError(#[source] CatalogClientError),

    /// An error that occurred while accessing the cache
    #[error("Unexpected error accessing cache: {0}")]
    CacheError(String),

    /// An error that occurred while calling nix build.
    #[error("Failed to call 'nix build'")]
    CallNixBuild(#[source] std::io::Error),

    #[error("Failed to write nix arguments to stdin")]
    WriteNixStdin(#[source] std::io::Error),

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
    // TODO: we should unravel the nix copy spaghetti in
    // try_substitute_published_package and give the actual reason `nix copy` failed
    #[error("Couldn't download package '{0}' for unknown reason")]
    BuildPublishedPackage(String),

    /// A custom package has been uploaded, but the current user hasn't configured
    /// a trusted public key that matches a signature of this package.
    #[error("Package '{0}' is not signed by a trusted key")]
    UntrustedPackage(String),

    #[error("authentication error")]
    Auth(#[source] AuthError),

    /// An error occurred while performing nix copy
    /// The contained string should be stderr, which may be a bit too much
    /// detail,
    /// but it will allow debugging for now.
    #[error("couldn't download package:\n{0}")]
    NixCopyError(String),

    // You will 99.9999% never see this in real life.
    #[error("internal error downloadng packages")]
    ThreadPanicked,

    // You will 99.9999% never see this in real life.
    #[error("internal error: mutex was poisoned")]
    PoisonedMutex,

    /// An unhandled condition was encountered in the lockfile.  One example is
    /// a package that is expected to be a base catalog package but the
    /// lockfile appears to be a custom package or vice versa.
    #[error("encountered an error interpreting the lockfile: {0}")]
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, Eq, PartialEq)]
pub struct BuildEnvOutputs {
    pub develop: BuiltStorePath,
    pub runtime: BuiltStorePath,
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
            ActivateMode::Dev => self.develop,
            ActivateMode::Run => self.runtime,
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
        client: &impl ClientTrait,
        lockfile: &Path,
        service_config_path: Option<PathBuf>,
    ) -> Result<BuildEnvOutputs, BuildEnvError>;
}

#[derive(Debug)]
pub struct NetRcAndAuth<A> {
    netrc_path: Option<TempPath>,
    auth: Arc<Mutex<A>>,
}

impl<A> NetRcAndAuth<A>
where
    A: AuthProvider,
{
    /// Returns the path to a populated netrc file, creating one if necessary.
    pub fn get_netrc_path(&mut self) -> Result<&TempPath, BuildEnvError> {
        if let Some(ref path) = self.netrc_path {
            Ok(path)
        } else {
            let path = self
                .auth
                .lock()
                .map_err(|_| BuildEnvError::PoisonedMutex)?
                .create_netrc()
                .map_err(BuildEnvError::Auth)?;
            self.netrc_path = Some(path);
            self.get_netrc_path()
        }
    }
}

pub struct BuildEnvNix<P, A> {
    gc_root_base_path: P,
    auth: A,
}

impl<P, A> BuildEnvNix<P, A>
where
    P: AsRef<Path>,
    A: AuthProvider,
{
    pub fn new(gc_root_base_path: P, auth: A) -> BuildEnvNix<P, A> {
        BuildEnvNix {
            gc_root_base_path,
            auth,
        }
    }

    /// Create a new gc root path in [Self::gc_root_base_path]
    /// with a unique prefix.
    fn new_gc_root_path(&self, prefix: impl AsRef<str>) -> PathBuf {
        self.gc_root_base_path.as_ref().join(prefix.as_ref())
    }

    fn base_command() -> Command {
        let mut nix_build_command = nix_base_command();
        // allow impure language features such as `builtins.storePath`,
        // and use the auto store (which is used by the preceding `realise` command)
        // TODO: formalize this in a config file,
        // and potentially disable other user configs (allowing specific overrides)
        nix_build_command.args(["--option", "pure-eval", "false"]);

        match std::env::var("_FLOX_NIX_STORE_URL").ok().as_deref() {
            None | Some("") => {
                debug!("using 'auto' store");
            },
            Some(store_url) => {
                debug!(%store_url, "overriding Nix store URL");
                nix_build_command.args(["--option", "store", store_url]);
            },
        }

        // we generally want to see more logs (we can always filter them out)
        nix_build_command.arg("--print-build-logs");

        nix_build_command
    }

    /// Check which storepaths of the environment to be built already exist
    /// and create GC roots for them.
    /// GC roots prevent those paths from being deleted by a concurrently running
    /// nix GC job, before they can be used as a dependency
    /// for the environment being built.
    fn pre_check_store_paths(
        &self,
        lockfile: &Lockfile,
        system: &System,
    ) -> Result<CheckedStorePaths, BuildEnvError> {
        let mut all_paths = Vec::new();
        for package in lockfile.packages.iter() {
            if package.system() != system {
                continue;
            }

            match package {
                LockedPackage::Catalog(locked) => all_paths.extend(locked.outputs.values()),
                LockedPackage::Flake(locked) => {
                    all_paths.extend(locked.locked_installable.outputs.values())
                },
                LockedPackage::StorePath(locked) => all_paths.extend([&locked.store_path]),
            }
        }

        let n_iters = 10;
        let sleep_duration = std::time::Duration::from_millis(100);
        let mut gc_root_err =
            BuildEnvError::Link("failed to create gc roots during build".to_string());
        for _ in 0..n_iters {
            let checked_store_paths = check_store_paths(&all_paths)?;
            match create_gc_root_in(
                &checked_store_paths.valid,
                self.new_gc_root_path("pre-checked-paths"),
            ) {
                Ok(_) => {
                    return Ok(checked_store_paths);
                },
                Err(BuildEnvError::Link(err)) => {
                    debug!(error = err, "failed to set one or more gc roots, retrying");
                    gc_root_err = BuildEnvError::Link(err.clone());
                },
                Err(e) => {
                    return Err(e);
                },
            }
            std::thread::sleep(sleep_duration);
        }
        Err(gc_root_err)
    }

    /// Realise all store paths of packages that are installed to the environment,
    /// for the given system.
    /// This goes through all packages in the lockfile and realises them with
    /// the appropriate method for the package type.
    ///
    /// See the individual realisation functions for more details.
    // todo: return actual store paths built,
    // necessary when building manifest builds.
    fn realise_lockfile(
        &self,
        client: &impl ClientTrait,
        lockfile: &Lockfile,
        system: &System,
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let mut base_catalog_pkgs = vec![];
        let mut custom_catalog_pkgs = vec![];
        let mut flake_pkgs = vec![];
        let mut store_path_pkgs = vec![];

        for package in lockfile.packages.iter() {
            if package.system() != system {
                continue;
            }

            // Look up the package entry in the manifest using the install_id
            let install_id = package.install_id();
            let manifest_package = lockfile
                .manifest
                .pkg_descriptor_with_id(install_id)
                .ok_or_else(|| {
                    BuildEnvError::Other(format!(
                        "Could not find package with install_id '{install_id}' in manifest"
                    ))
                })?;

            match package {
                LockedPackage::Catalog(pkg) => {
                    if manifest_package.is_from_custom_catalog() {
                        custom_catalog_pkgs.push((manifest_package, pkg));
                    } else {
                        base_catalog_pkgs.push(pkg);
                    }
                },
                LockedPackage::Flake(pkg) => flake_pkgs.push(pkg),
                LockedPackage::StorePath(pkg) => store_path_pkgs.push(pkg),
            }
        }

        self.realise_base_catalog_pkgs(&base_catalog_pkgs, pre_checked_store_paths)?;
        self.realise_custom_catalog_pkgs(client, &base_catalog_pkgs, pre_checked_store_paths)?;
        self.realise_store_path_pkgs(&store_path_pkgs, pre_checked_store_paths)?;
        for flake in flake_pkgs.iter() {
            self.realise_flakes(flake, pre_checked_store_paths)?;
        }
        Ok(())
    }

    fn realise_custom_catalog_pkgs(
        &self,
        client: &impl ClientTrait,
        pkgs: &[&LockedPackageCatalog],
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let all_store_paths = pkgs
            .iter()
            .flat_map(|pkg| pkg.outputs.values().map(|sp| sp.to_string()))
            .collect::<Vec<_>>();
        let store_locations = client
            .get_store_info(all_store_paths)
            .block_on()
            .map_err(BuildEnvError::CatalogError)?;
        let no_netrc_is_error = self.auth.token().is_none();
        let netrc_path = self.auth.try_create_netrc();
        let borrowed_netrc_path = netrc_path.as_ref();
        let span = Span::current();
        let gc_root_base_dir = self.gc_root_base_path.as_ref();
        let borrowed_store_locations = &store_locations;
        std::thread::scope(move |s| {
            let mut thread_handles = vec![];
            for pkg in pkgs.iter() {
                for store_path in pkg.outputs.values() {
                    let all_valid_in_pre_checked = pkg
                        .outputs
                        .values()
                        .all(|path| pre_checked_store_paths.valid(path).unwrap_or_default());
                    if all_valid_in_pre_checked {
                        continue;
                    }
                    let inner_span = span.clone();
                    let handle = s.spawn(move || {
                        Self::try_substitute_single_custom_catalog_pkg_store_path(
                            store_path,
                            &pkg.install_id,
                            &pkg.attr_path,
                            gc_root_base_dir,
                            borrowed_store_locations,
                            no_netrc_is_error,
                            borrowed_netrc_path,
                            inner_span,
                        )
                    });
                    thread_handles.push(handle);
                }
            }
            let mut thread_panicked = false;
            for h in thread_handles {
                thread_panicked |= h.join().is_err();
            }
            if thread_panicked {
                return Err(BuildEnvError::ThreadPanicked);
            }
            Ok::<(), BuildEnvError>(())
        })
        .map_err(|_| BuildEnvError::ThreadPanicked)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn try_substitute_single_custom_catalog_pkg_store_path(
        store_path: &str,
        install_id: &str,
        attr_path: &str,
        gc_root_base_dir: &Path,
        store_locations: &HashMap<String, Vec<StoreInfo>>,
        no_netrc_is_error: bool,
        maybe_netrc_path: Option<&PathBuf>,
        parent_span: Span,
    ) -> Result<(), BuildEnvError> {
        let mut auth_error = None;
        let locations =
            store_locations
                .get(store_path)
                .ok_or(BuildEnvError::NoPackageStoreLocation(
                    install_id.to_string(),
                ))?;
        let span = info_span!(
            parent: parent_span.clone(),
            "substitute custom catalog package",
            progress = format!("Downloading '{}'", attr_path)
        );
        let _ = span.enter();
        for location in locations {
            // nix copy
            let mut copy_command = nix_base_command();
            let location_url = match &location.url {
                Some(url) => url,
                None => {
                    return Err(BuildEnvError::NixCopyError(format!(
                        "Missing store location URL for package '{}'",
                        install_id
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
            copy_command.arg("--from").arg(location_url).arg(store_path);
            debug!(cmd=%copy_command.display(), "trying to copy published package");
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
                // If we failed, log the error and try the next location.
                debug!(%store_path, %location_url, %stderr, "Failed to copy package from store");
            } else {
                // If we succeeded, then we can continue with the next path
                debug!(%store_path, %location_url, "Succesfully copied package from store");

                // TODO: there is a real but very short period between the successful copy
                // and setting the gc root in which the path _could_ be collected as garbage,
                // which we could guard against by wrapping the copy/link/check in a loop.
                // At this point its not clear whether that is worth the additional complexity.
                let filename = Path::new(store_path)
                    .file_name()
                    .ok_or_else(|| BuildEnvError::Link(format!("Invalid store path {store_path}")))?
                    .to_string_lossy();
                create_gc_root_in(
                    [store_path],
                    gc_root_base_dir.join(format!("by-store-path/{filename}")),
                )?;
            }
        }
        if let Some(err) = auth_error {
            Err(err)
        } else {
            Ok(())
        }
    }

    /// Try to substitute a published package by copying it from an associated store.
    ///
    /// Query the associated store(s) that contain the package from the catalog.
    /// Then attempt to download the package closure from each store in order,
    /// until successful.
    /// Returns `true` if all outputs were found and downloaded, `false` otherwise.
    ///
    /// If a path is found, we attempt to create a temproot for it in [Self::gc_root_base_path].
    fn try_substitute_published_pkg(
        &self,
        client: &impl ClientTrait,
        locked: &LockedPackageCatalog,
    ) -> Result<bool, BuildEnvError> {
        debug!(
            install_id = locked.install_id,
            "trying to substitute published package"
        );
        // TODO - The API call accepts multiple, so an optimization is to
        // collect these for the whole lockfile ahead of time and ask for them
        // all at once.
        let paths: Vec<String> = locked.outputs.values().map(|s| s.to_string()).collect();
        let store_locations = client
            .get_store_info(paths)
            .block_on()
            .map_err(BuildEnvError::CatalogError)?;

        // Try downloading each output from the store location provided.  If we
        // are missing store info for any, we should return false.
        // TODO - It is possible not _all_ are missing. For now, we'll just
        // assume they are all missing, noting published packages only have one
        // output currently.  Also to note: if all the outputs were valid
        // locally, we would not get here as that check happens before this is
        // called within `realise_nixpkgs`.
        let mut netrc_path = None;
        'path_loop: for (path, locations) in store_locations.iter() {
            let mut auth_error = None;
            // If there are no locations
            if locations.is_empty() {
                return Err(BuildEnvError::NoPackageStoreLocation(
                    locked.install_id.clone(),
                ));
            }
            for location in locations {
                // nix copy
                let mut copy_command = nix_base_command();
                let location_url = match &location.url {
                    Some(url) => url,
                    None => {
                        return Err(BuildEnvError::NixCopyError(format!(
                            "Missing store location URL for package '{}'",
                            locked.install_id
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
                        // Don't attempt to get the token until we need it,
                        // and cache the netrc path so that we don't make the same
                        // file over and over again.
                        match netrc_path {
                            Some(Ok(ref path)) => {
                                copy_command.arg("--netrc-file").arg(path);
                            },
                            Some(Err(_)) => {
                                // Do nothing and hope that a later `location` doesn't
                                // need a token since at some point in the past we
                                // needed one, looked for it, and didn't get one.
                                // Note that we don't continue because
                                // store_needs_auth isn't necessarily correct.
                            },
                            None => {
                                let maybe_path =
                                    self.auth.create_netrc().map_err(BuildEnvError::Auth);
                                if let Ok(ref path) = maybe_path {
                                    copy_command.arg("--netrc-file").arg(path);
                                }
                                netrc_path = Some(maybe_path);
                            },
                        }
                    }
                }
                copy_command.arg("--from").arg(location_url).arg(path);
                debug!(cmd=%copy_command.display(), "trying to copy published package");
                let output = copy_command
                    .output()
                    .map_err(|e| BuildEnvError::CacheError(e.to_string()))?;
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.contains("because it lacks a signature by a trusted key") {
                        return Err(BuildEnvError::UntrustedPackage(locked.install_id.clone()));
                    }
                    // We're expecting errors for netrc type auth, but not for
                    // catalog provided auth.
                    if location.auth.is_some() {
                        auth_error = Some(BuildEnvError::NixCopyError(stderr.to_string()));
                    }
                    // If we failed, log the error and try the next location.
                    debug!(%path, %location_url, %stderr, "Failed to copy package from store");
                } else {
                    // If we succeeded, then we can continue with the next path
                    debug!(%path, %location_url, "Succesfully copied package from store");

                    // TODO: there is a real but very short period between the successful copy
                    // and setting the gc root in which the path _could_ be collected as garbage,
                    // which we could guard against by wrapping the copy/link/check in a loop.
                    // At this point its not clear whether that is worth the additional complexity.
                    let filename = Path::new(path)
                        .file_name()
                        .ok_or_else(|| BuildEnvError::Link(format!("Invalid store path {path}")))?
                        .to_string_lossy();
                    create_gc_root_in(
                        [path],
                        self.new_gc_root_path(format!("by-store-path/{filename}")),
                    )?;

                    continue 'path_loop;
                }
            }
            // If we get here, we could not download the current path from anywhere
            debug!(%path, "Failed to copy path from any provided location");
            // At some point we needed an authentication token
            // and didn't find one.
            if let Some(Err(e)) = netrc_path {
                return Err(e);
            }

            // At some point we tried to authenticate using catalog provided
            // credentials and there was an error
            if let Some(e) = auth_error {
                return Err(e);
            }

            return Ok(false);
        }
        Ok(true)
    }

    fn realise_base_catalog_pkgs(
        &self,
        pkgs: &[&LockedPackageCatalog],
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let gc_root_base_path = self.gc_root_base_path.as_ref();
        let span = Span::current();
        std::thread::scope(move |s| {
            let mut thread_handles = vec![];
            for locked_pkg in pkgs {
                let inner_span = span.clone();
                let handle = s.spawn(move || {
                    Self::realise_single_base_catalog_pkg(
                        locked_pkg,
                        gc_root_base_path,
                        pre_checked_store_paths,
                        inner_span,
                    )
                });
                thread_handles.push(handle);
            }
            let mut thread_panicked = false;
            for h in thread_handles {
                thread_panicked |= h.join().is_err();
            }
            if thread_panicked {
                return Err(BuildEnvError::ThreadPanicked);
            }
            Ok::<(), BuildEnvError>(())
        })
        .map_err(|_| BuildEnvError::ThreadPanicked)?;
        Ok(())
    }

    fn realise_single_base_catalog_pkg(
        locked_pkg: &LockedPackageCatalog,
        gc_root_base_dir: &Path,
        pre_checked_store_paths: &CheckedStorePaths,
        span: Span,
    ) -> Result<(), BuildEnvError> {
        // Check if all store paths are valid, or can be substituted.
        let all_valid_in_pre_checked = locked_pkg
            .outputs
            .values()
            .all(|path| pre_checked_store_paths.valid(path).unwrap_or_default());

        // If all store paths are already valid, we can return early.
        if all_valid_in_pre_checked {
            return Ok(());
        }

        let gc_root_path = gc_root_base_dir.join(format!("by-iid/{}", locked_pkg.install_id));

        // Check if store paths have _become_ valid in the meantime or can be substituted.
        let all_valid_after_build_or_substitution = {
            let span = info_span!(
                parent: span.clone(),
                "substitute catalog package",
                progress = format!("Downloading '{}'", locked_pkg.attr_path)
            );
            span.in_scope(|| {
                Self::check_store_path_with_substituters(&gc_root_path, locked_pkg.outputs.values())
            })?
        };

        // If all store paths are valid after substitution, we can return early.
        if all_valid_after_build_or_substitution {
            return Ok(());
        }

        let installable = {
            let mut locked_url = locked_pkg.locked_url.to_string();
            if let Some(revision_suffix) = locked_url.strip_prefix(NIXPKGS_CATALOG_URL_PREFIX) {
                locked_url = format!("{FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE}/{revision_suffix}");
            } else {
                return Err(BuildEnvError::Other(format!(
                    "Locked package '{}' is a base catalog package, but the locked url '{}' does not start with the expected prefix '{}'",
                    locked_pkg.install_id, locked_pkg.locked_url, NIXPKGS_CATALOG_URL_PREFIX
                )));
            }

            // build all out paths
            let attrpath = format!(
                "legacyPackages.{}.{}^*",
                locked_pkg.system, locked_pkg.attr_path
            );

            format!("{}#{}", locked_url, attrpath)
        };

        let _span = info_span!(
            parent: span,
            "build from catalog",
            progress = format!("Building '{}' from source", locked_pkg.attr_path)
        )
        .entered();

        let mut nix_build_command = Self::base_command();

        nix_build_command.args(["--option", "extra-plugin-files", &*NIX_PLUGINS]);

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--out-link");
        nix_build_command.arg(&gc_root_path);
        nix_build_command.arg(&installable);

        debug!(%installable, cmd=%nix_build_command.display(), "building catalog package");

        let output = nix_build_command
            .output()
            .map_err(BuildEnvError::CallNixBuild)?;

        if !output.status.success() {
            return Err(BuildEnvError::Realise2 {
                install_id: locked_pkg.install_id.clone(),
                message: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }

        Ok(())
    }

    /// Realise a package from the (nixpkgs) catalog.
    /// [LockedPackageCatalog] is a locked package from the catalog.
    /// The package is realised by checking if the store paths are valid,
    /// and otherwise building the package to create valid store paths.
    /// Packages are built by
    /// 1. translating the locked url to a `flox-nixpkgs` url,
    ///    which is a bridge to the Flox hosted mirror of nixpkgs flake
    ///    <https://github.com/flox/nixpkgs>, which enables building packages
    ///    without common evaluation checks, such as unfree and broken.
    /// 2. constructing the attribute path to build the package,
    ///    i.e. `legacyPackages.<locked system>.<attr_path>`,
    ///    as [LockedPackageCatalog::attr_path] is incomplete.
    /// 3. building the package with essentially
    ///    `nix build <flox-nixpkgs-url>#<resolved attr path>^*`,
    ///    which will realise the locked output paths.
    ///    We set `--option pure-eval true` to improve reproducibility
    ///    of the locked outputs, and allow the use of the eval-cache
    ///    to avoid costly re-evaluations.
    ///    When building or substituting sets GC temp roots for the _new_ paths.
    ///
    /// IMPORTANT/TODO: As custom catalogs, with non-nixpkgs packages are in development,
    /// this function is currently assumes that the package is from the nixpkgs base-catalog.
    /// Currently the type is distinguished by the [LockedPackageCatalog::locked_url].
    /// If this does not indicate a nixpkgs package, the function will currently panic!
    fn realise_nixpkgs(
        &self,
        client: &impl ClientTrait,
        manifest_package: &ManifestPackageDescriptor,
        locked: &LockedPackageCatalog,
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        // Check if all store paths are valid, or can be substituted.
        let all_valid_in_pre_checked = locked
            .outputs
            .values()
            .all(|path| pre_checked_store_paths.valid(path).unwrap_or_default());

        // If all store paths are already valid, we can return early.
        if all_valid_in_pre_checked {
            return Ok(());
        }

        // Check if store paths have _become_ valid in the meantime or can be substituted.
        let all_valid_after_build_or_substitution = {
            let span = info_span!(
                "substitute catalog package",
                progress = format!("Downloading '{}'", locked.attr_path)
            );
            span.in_scope(|| {
                Self::check_store_path_with_substituters(
                    &self.new_gc_root_path(format!("by-iid/{}", &locked.install_id)),
                    locked.outputs.values(),
                )
            })?
        };

        // If all store paths are valid after substitution, we can return early.
        if all_valid_after_build_or_substitution {
            return Ok(());
        }

        // TODO: less flimsy handling of building published packages
        // 1. custom catalogs are distinguished from nixpkgs catalog
        //    only by the prefix of the url field.
        // 2. custom packages cannot be referred to by nix installable
        // 3. from this point onward the whole buildprocess diverges between both types of packages
        let installable = {
            let mut locked_url = locked.locked_url.to_string();

            if !manifest_package.is_from_custom_catalog() {
                if let Some(revision_suffix) = locked_url.strip_prefix(NIXPKGS_CATALOG_URL_PREFIX) {
                    locked_url = format!("{FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE}/{revision_suffix}");
                } else {
                    return Err(BuildEnvError::Other(format!(
                        "Locked package '{}' is a base catalog package, but the locked url '{}' does not start with the expected prefix '{}'",
                        locked.install_id, locked.locked_url, NIXPKGS_CATALOG_URL_PREFIX
                    )));
                }
            } else {
                debug!(?locked.attr_path, "Trying to substitute published package");
                let span = info_span!(
                    "substitute custom catalog package",
                    progress = format!("Downloading '{}'", locked.attr_path)
                );
                let all_found =
                    span.in_scope(|| self.try_substitute_published_pkg(client, locked))?;
                // We asked for all the outputs for the package, got store info for
                // each, and were able to substitute them all.  If so, then we're done here.
                if all_found {
                    return Ok(());
                };
                return Err(BuildEnvError::BuildPublishedPackage(
                    locked.install_id.clone(),
                ));
            }

            // build all out paths
            let attrpath = format!("legacyPackages.{}.{}^*", locked.system, locked.attr_path);

            format!("{}#{}", locked_url, attrpath)
        };

        let _span = info_span!(
            "build from catalog",
            progress = format!("Building '{}' from source", locked.attr_path)
        )
        .entered();

        let mut nix_build_command = Self::base_command();

        nix_build_command.args(["--option", "extra-plugin-files", &*NIX_PLUGINS]);

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--out-link");
        nix_build_command.arg(self.new_gc_root_path(format!("by-iid/{}", locked.install_id)));
        nix_build_command.arg(&installable);

        debug!(%installable, cmd=%nix_build_command.display(), "building catalog package");

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

    /// Realise a package from a flake.
    /// [LockedPackageFlake] is a locked package from a flake installable.
    /// The package is realised by checking if the store paths are valid,
    /// and otherwise building the package to create valid store paths.
    /// Packages are built by optimistically joining the flake url and attr path,
    /// which has been previously evaluated successfully during locking,
    /// and building the package with essentially `nix build <flake-url>#<attr-path>^*`.
    /// We set `--option pure-eval true` to avoid improve reproducibility,
    /// and allow the use of the eval-cache to avoid costly re-evaluations.
    /// When building or substituting sets GC temp roots for the _new_ paths.
    #[instrument(skip(self), fields(progress = format!("Realising flake package '{}'", locked.install_id)))]
    fn realise_flakes(
        &self,
        locked: &LockedPackageFlake,
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let all_valid_in_pre_checked = locked
            .locked_installable
            .outputs
            .values()
            .all(|path| pre_checked_store_paths.valid(path).unwrap_or_default());

        // check if all store paths are valid, if so, return without eval
        if all_valid_in_pre_checked {
            return Ok(());
        }

        // check if store paths have _become_ valid in the meantime
        let all_valid = self.check_store_path(locked.locked_installable.outputs.values())?;
        if all_valid {
            return Ok(());
        }

        let mut nix_build_command = Self::base_command();

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
        nix_build_command.arg("--out-link");
        nix_build_command.arg(self.new_gc_root_path(format!("by-iid/{}", locked.install_id)));
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
        gc_root_base_path: &Path,
        pre_checked_store_paths: &CheckedStorePaths,
        parent_span: Span,
    ) -> Result<(), BuildEnvError> {
        let pre_valid = pre_checked_store_paths
            .valid(&locked.store_path)
            .unwrap_or_default();

        let valid = pre_valid || {
            let span = info_span!(
                parent: parent_span,
                "substitute store path",
                progress = format!("Downloading '{}'", locked.store_path)
            );
            span.in_scope(|| {
                Self::check_store_path_with_substituters(
                    &gc_root_base_path.join(format!("by-store-path/{}", &locked.store_path)),
                    [&locked.store_path],
                )
            })?
        };

        if !valid {
            return Err(BuildEnvError::Realise2 {
                install_id: locked.install_id.clone(),
                message: format!("'{}' is not available", locked.store_path),
            });
        }
        Ok(())
    }

    fn realise_store_path_pkgs(
        &self,
        pkgs: &[&LockedPackageStorePath],
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let gc_root_base_path = self.gc_root_base_path.as_ref();
        let span = Span::current();
        std::thread::scope(move |s| {
            let mut thread_handles = vec![];
            for locked_pkg in pkgs {
                let inner_span = span.clone();
                let handle = s.spawn(move || {
                    Self::realise_single_store_path(
                        locked_pkg,
                        gc_root_base_path,
                        pre_checked_store_paths,
                        inner_span,
                    )
                });
                thread_handles.push(handle);
            }
            let mut thread_panicked = false;
            for h in thread_handles {
                thread_panicked |= h.join().is_err();
            }
            if thread_panicked {
                return Err(BuildEnvError::ThreadPanicked);
            }
            Ok::<(), BuildEnvError>(())
        })
        .map_err(|_| BuildEnvError::ThreadPanicked)?;
        Ok(())
    }

    /// Check if the given store paths exists in the configured nix store.
    /// Substitute store paths if necessary and possible.
    /// Sets a gc root in [Self::gc_root_base_path] on success.
    ///
    /// This methods is expected to be called _for all outputs of a derivation_
    /// iff any output of the derivation has formerly been identified as invalid
    /// by [Self::pre_check_store_paths].
    ///
    /// To avoid GC of substituted paths, pass an `--out-links` argument,
    /// to create "temproots".
    fn check_store_path_with_substituters(
        gc_root_path: &Path,
        paths: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<bool, BuildEnvError> {
        let mut cmd = Self::base_command();
        cmd.arg("build");
        cmd.arg("--out-link");
        cmd.arg(gc_root_path);
        // cmd.arg(self.new_gc_root_path(format!("by-iid/{install_id}")));
        cmd.args(paths);

        debug!(cmd=%cmd.display(), "checking store paths, including substituters");

        let success = cmd
            .output()
            .map_err(BuildEnvError::CallNixBuild)?
            .status
            .success();

        Ok(success)
    }

    /// Check if the given store paths _exists_ on the filesystem.
    ///
    /// If the store paths do not exist,
    /// the function will fall back to querying the nix store for the store paths.
    /// Formerly, this function checked the store paths with `nix path-info` immediately,
    /// which would also ensure the integrity of the references of the store paths.
    /// However, the runtime profile of the `nix path-info` command
    /// has significant overhead for large environments.
    /// 50ms to 100ms per package in an environment of 50 packages,
    /// is very noticeable.
    /// To address this we replace the nix call with a number of `stat`
    /// calls for the paths that are checked, with the optimistic assumption
    /// that if a path exists, it and its references are valid.
    /// If they are not, we fall back to the nix call,
    /// which allows checking against alternative stores.
    fn check_store_path(
        &self,
        paths: impl IntoIterator<Item = impl AsRef<str> + Hash + Eq>,
    ) -> Result<bool, BuildEnvError> {
        Ok(check_store_paths(paths)?.all_valid())
    }

    /// Build the environment by evaluating and building
    /// the `buildenv.nix` expression.
    ///
    /// The `buildenv.nix` reads the lockfile and composes
    /// an environment derivation, with outputs for the `develop` and `runtime` modes,
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
    ) -> Result<BuildEnvOutputs, BuildEnvError> {
        let mut nix_build_command = Self::base_command();
        nix_build_command.args(["build", "--no-link", "--offline", "--json"]);
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
            return Ok(build_env_result.outputs);
        }

        // Preexisting store paths produced by the build may have-new been (partially) swept away.
        // In that case the above `nix build` only documents the _new_ outputs.
        // A second build with the same arguments will be fully substituted and contain all outputs.
        //
        // We only try this once because the weindow for paths to disappear between the last build
        // and this one is particularly short, incorrect output is now reliably wrong
        // and should be propagated up.
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
            .inspect_err(|_| debug!("failed to deserialize output on second try"))
            .map_err(|err| BuildEnvError::ReadOutputs {
                output: String::from_utf8_lossy(&output.stdout).to_string(),
                err,
            })?;
        let outputs = build_env_result.outputs;
        Ok(outputs)
    }
}

impl<P, A> BuildEnv for BuildEnvNix<P, A>
where
    P: AsRef<Path>,
    A: AuthProvider,
{
    #[instrument(skip_all, fields(progress = "Building environment"))]
    fn build(
        &self,
        client: &impl ClientTrait,
        lockfile_path: &Path,
        service_config_path: Option<PathBuf>,
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
        if let Some(ref systems) = lockfile.manifest.options.systems
            && !systems.contains(&env!("NIX_TARGET_SYSTEM").to_string())
        {
            return Err(BuildEnvError::LockfileIncompatible {
                systems: systems.clone(),
            });
        }

        // Check all store paths of the lockfile packages,
        // for validity _in the current store_ as a single bulk operation.
        // This is a performance optimization to avoid the overhead
        // of individual `nix path-info` calls per package.
        // `nix path-info` takes about 50ms to 100ms per call,
        // most of which is overhead, since empirically
        // the command shows relatively constant runtime with the number of paths.
        // However, for large environments with many packages,
        // the individual calls add up.
        // Instead we use `nix path-info --stdin` to check all paths at once,
        // and pass on the result as a cache to the `realise` step,
        // which can query the validity of paths efficiently on a per package basis.
        let pre_checked_store_paths =
            self.pre_check_store_paths(&lockfile, &env!("NIX_TARGET_SYSTEM").to_string())?;

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
        // TODO:
        // Eventually we want to retrieve a record of the built store paths,
        // to pass explicitly to the `buildenv.nix` expression.
        // This will prevent failures due to e.g. non-deterministic,
        // non-sandboxed manifest builds which may produce different store paths,
        // than previously locked in the lockfile.
        self.realise_lockfile(
            client,
            &lockfile,
            &env!("NIX_TARGET_SYSTEM").to_string(),
            &pre_checked_store_paths,
        )?;

        // Build the lockfile by evaluating and building the `buildenv.nix` expression.
        let outputs = self.call_buildenv_nix(lockfile_path, service_config_path)?;

        Ok(outputs)
    }
}

/// A helper struct to keep track of the store paths that have been checked
/// and which of them are valid.
#[derive(Clone, Debug, Default)]
struct CheckedStorePaths {
    /// The store paths that have been checked.
    /// The validity of `CheckedStorePaths` is limited to the store paths actually checked.
    /// I.e. if a store path was not checked, it can not be considered valid nor invalid.
    checked: HashSet<String>,
    /// The store paths that have been checked and are valid.
    /// The construction of [CheckedStorePaths], i.e. [check_store_paths]
    /// ensures that `valid ⊆ checked`
    valid: HashSet<String>,
}

impl CheckedStorePaths {
    /// Check whether all checked store paths are valid.
    fn all_valid(&self) -> bool {
        self.checked.len() == self.valid.len()
    }

    /// Check whether a store path is valid.
    /// If the store path has not been checked, the function will return `None`.
    fn valid(&self, path: impl AsRef<str>) -> Option<bool> {
        self.checked(&path)
            .then(|| self.valid.contains(path.as_ref()))
    }

    /// Check whether a store path has been checked.
    fn checked(&self, path: impl AsRef<str>) -> bool {
        self.checked.contains(path.as_ref())
    }
}

/// Check the validity of store paths in the nix store,
/// without attempting to build or substitute them.
/// The [CheckedStorePaths] struct returned by this function
/// will be used to inform the various `realise_*` functions,
/// whether a package needs to be built or substituted.
///
/// SAFTETY: [CheckedStorePaths] poses the risk of TOCTOU issues,
/// especially when held for a long time.
/// We acknowledge, that store paths might be _created_,
/// after [CheckedStorePaths] is created;
/// Consumers may check the validity of invalid store paths again
/// before attempting expensive operations like building or substituting,
/// in case the paths have been created separately in the meantime.
/// However, we assume that the store paths are not being _deleted_,
/// which would invalidate the [CheckedStorePaths] struct.
/// Concurrent deletion of store paths is a rare event,
/// but can lead to intermittent build failures.
/// Since the nix store is not in our control, or transactional,
/// we accept this risk as a trade-off for performance and try to mitigate it
/// by limiting the scope/lifetime of the [CheckedStorePaths] struct.
fn check_store_paths(
    paths: impl IntoIterator<Item = impl AsRef<str> + Eq + Hash>,
) -> Result<CheckedStorePaths, BuildEnvError> {
    let mut command = nix::nix_base_command();
    command.stdin(Stdio::piped());
    command.stderr(Stdio::null());
    command.stdout(Stdio::piped());
    command.args(["path-info", "--offline", "--stdin"]);

    debug!(cmd=%command.display(), "bulk checking validity of store_paths (paths passed to stdin)");

    let mut child = command.spawn().map_err(BuildEnvError::CallNixBuild)?;
    let stdin = child.stdin.as_mut().unwrap();

    let paths = paths
        .into_iter()
        .map(|p| p.as_ref().to_string())
        .collect::<HashSet<_>>();

    for path in paths.iter() {
        trace!(%path, "checking validity of store path");
        writeln!(stdin, "{path}").map_err(BuildEnvError::WriteNixStdin)?;
    }

    stdin.flush().map_err(BuildEnvError::WriteNixStdin)?;

    let output = child
        .wait_with_output()
        .map_err(BuildEnvError::CallNixBuild)?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let valid_paths = stdout
        .lines()
        .map(|p| p.to_string())
        .collect::<HashSet<_>>();

    Ok(CheckedStorePaths {
        checked: paths,
        valid: valid_paths,
    })
}

/// Create GC roots for the given store paths.
/// All provided paths must exist and be valid.
/// It's recommended to run [check_store_paths] to verify the validity of the paths.
/// If a gc process may be runnin in the background there is a short time
/// in which paths returned as valid by [check_store_paths] are deleted
/// before a temproot can be set.
/// In that case checking and setting gc-roots can be retried safely
/// until setting gc roots succeeds.
///
/// Gc roots are created in the provided `gc_root_base_dir`
/// with a unique prefix _per call_ to this function:
///
/// ```text
/// <basedir>/
///     gc-root.rqw2rr3-1
///     gc-root.rqw2rr3-2
///     gc-root.rqw2rr3-3
///     gc-root.i1343ca-1   # separate call to create_gc_root_in()
/// ```
///
/// as they would otherwise override exiting roots.
/// It is advisable to place the <basedir> under `/tmp`
/// to allow paths to be GC'd eventually if they are otherwise unused.
pub(crate) fn create_gc_root_in(
    paths: impl IntoIterator<Item = impl AsRef<Path>>,
    gc_root_prefix: impl AsRef<Path>,
) -> Result<(), BuildEnvError> {
    let paths = paths
        .into_iter()
        .map(|p| p.as_ref().to_string_lossy().into_owned())
        .collect::<HashSet<_>>();

    if paths.is_empty() {
        debug!("no paths to create gc roots for, skipping");
        return Ok(());
    }

    let mut command = nix::nix_base_command();
    command.stdin(Stdio::piped());
    command.stderr(Stdio::piped());
    command.stdout(Stdio::piped());
    command.args(["build", "--stdin"]);
    // avoid substitution or builds
    command.args(["--offline", "-j", "0"]);
    command.arg("--out-link");
    command.arg(gc_root_prefix.as_ref());

    let paths_arg = paths
        .iter()
        .map(|s| s.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    debug!(
        cmd = format!("echo '{}' | {}", paths_arg, command.display()),
        "bulk setting gc roots for store_paths"
    );

    let mut child = command.spawn().map_err(BuildEnvError::CallNixBuild)?;
    let stdin = child.stdin.as_mut().unwrap();

    for path in paths.iter() {
        trace!(%path, "setting gc root for store path");
        writeln!(stdin, "{path}").map_err(BuildEnvError::WriteNixStdin)?;
    }

    stdin.flush().map_err(BuildEnvError::WriteNixStdin)?;

    let output = child
        .wait_with_output()
        .map_err(BuildEnvError::CallNixBuild)?;

    if !output.status.success() {
        return Err(BuildEnvError::Link(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod test_helpers {
    use tempfile::TempDir;

    use super::*;
    use crate::providers::auth::Auth;

    pub(super) fn buildenv_instance() -> BuildEnvNix<TempDir, Auth> {
        let tempdir = TempDir::new().unwrap();
        let auth = Auth::from_tempdir_and_token(TempDir::new().unwrap(), None);
        BuildEnvNix::new(tempdir, auth)
    }
}

#[cfg(test)]
mod realise_nixpkgs_tests {

    use test_helpers::buildenv_instance;

    use super::*;
    use crate::models::lockfile;
    use crate::models::manifest::typed::PackageDescriptorCatalog;
    use crate::providers::catalog::{GENERATED_DATA, MockClient, StoreInfo, StoreInfoResponse};
    use crate::providers::nix::test_helpers::known_store_path;

    /// Read a single locked package for the current system from a mock lockfile.
    /// This is a helper function to avoid repetitive boilerplate in the tests.
    /// The lockfiles are generated by the `mk_data`, by using `flox lock-manifest`.
    /// Returns a tuple of (LockedPackageCatalog, ManifestPackageDescriptor).
    fn locked_package_catalog_from_mock(
        mock_lockfile: impl AsRef<Path>,
    ) -> (LockedPackageCatalog, ManifestPackageDescriptor) {
        let lockfile =
            lockfile::Lockfile::read_from_file(&CanonicalPath::new(mock_lockfile).unwrap())
                .expect("failed to read lockfile");
        let locked_package = lockfile
            .packages
            .into_iter()
            .find_map(|package| match package {
                LockedPackage::Catalog(locked) if locked.system == env!("NIX_TARGET_SYSTEM") => {
                    Some(locked)
                },
                _ => None,
            })
            .expect("no locked package found");

        let manifest_package = lockfile
            .manifest
            .pkg_descriptor_with_id(&locked_package.install_id)
            .expect("no manifest package found");

        (locked_package, manifest_package.clone())
    }

    fn locked_published_package(
        store_path: Option<&str>,
    ) -> (LockedPackageCatalog, ManifestPackageDescriptor) {
        let (mut locked_package, _) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));

        // make a new custom manifest descriptor such that we determine this is a published package
        let manifest_package = ManifestPackageDescriptor::Catalog(PackageDescriptorCatalog {
            pkg_path: "custom/hello".to_string(),
            pkg_group: Some("my_group".to_string()),
            priority: None,
            version: None,
            systems: None,
            outputs: None,
        });

        locked_package.attr_path = "hello".to_string();
        locked_package.locked_url =
            "github:super/custom/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string();

        // replace the store path with a known invalid one, to trigger an attempt to rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid";
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            store_path.unwrap_or(invalid_store_path).to_string(),
        );
        (locked_package, manifest_package)
    }

    /// When a package is not available in the store, it should be built from its derivation.
    /// This test sets a known invalid store path to trigger a rebuild of the 'hello' package.
    /// Since we're unable to provide unique store paths for each test run,
    /// this test is only indicative that we _actually_ build the package.
    #[test]
    fn nixpkgs_build_reproduce_if_invalid() {
        let (mut locked_package, manifest_package) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // Note: Packages from the catalog are always possibly present already
        // especially if they are built by a previous run of the test suite.
        // hence we can't check if they are invalid before building.

        let buildenv = buildenv_instance();

        let result = buildenv.realise_nixpkgs(
            &client,
            &manifest_package,
            &locked_package,
            &Default::default(),
        );
        assert!(result.is_ok());

        // Note: per the above this may be incidentally true
        assert!(buildenv.check_store_path([original_store_path]).unwrap());
    }

    /// When a package is available in the store, it should not be evaluated or built.
    /// This test sets the attribute path to a known bad value,
    /// to ensure that the build will fail if buildenv attempts to evaluate the package.
    #[test]
    fn nixpkgs_skip_eval_if_valid() {
        let (mut locked_package, manifest_package) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new();

        // build the package to ensure it is in the store
        let buildenv = buildenv_instance();
        buildenv
            .realise_nixpkgs(
                &client,
                &manifest_package,
                &locked_package,
                &Default::default(),
            )
            .expect("'hello' package should build");

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();
        buildenv
            .realise_nixpkgs(
                &client,
                &manifest_package,
                &locked_package,
                &Default::default(),
            )
            .expect("'hello' package should be realised without eval/build");
    }

    /// Realising a nixpkgs package should fail if the output is not valid
    /// and cannot be built.
    /// Here we are testing the case where the attribute fails to evaluate.
    /// Generally we expect pacakges from the catalog to be able to evaluate,
    /// iff the catalog server was able to evaluate them before.
    /// This test is a catch-all for all kinds of eval failures.
    /// Eval failures for **unfree** and **broken** packages should be prevented,
    /// which is tested in the tests below.
    #[test]
    fn nixpkgs_eval_failure() {
        let (mut locked_package, manifest_package) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();

        let buildenv = buildenv_instance();
        let result = buildenv.realise_nixpkgs(
            &client,
            &manifest_package,
            &locked_package,
            &Default::default(),
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
        let (mut locked_package, manifest_package) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello-unfree-lock.yaml"));
        let client = MockClient::new();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = buildenv_instance();
        let result = buildenv.realise_nixpkgs(
            &client,
            &manifest_package,
            &locked_package,
            &Default::default(),
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
        let (mut locked_package, manifest_package) =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/tabula-lock.yaml"));
        let client = MockClient::new();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = buildenv_instance();
        let result = buildenv.realise_nixpkgs(
            &client,
            &manifest_package,
            &locked_package,
            &Default::default(),
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
        let mut client = MockClient::new();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };
        let fake_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        resp.items.insert(fake_store_path.clone(), vec![]);
        client.push_store_info_response(resp);

        let buildenv = buildenv_instance();
        let subst_resp = buildenv.try_substitute_published_pkg(&client, &locked_package);
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
        let mut client = MockClient::new();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };

        // This is a trick for a known storepath
        resp.items.insert(real_storepath_str.to_string(), vec![
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
        client.push_store_info_response(resp);

        let buildenv = buildenv_instance();
        let subst_resp = buildenv
            .try_substitute_published_pkg(&client, &locked_package)
            .unwrap();
        assert!(subst_resp);
    }

    #[test]
    fn nixpkgs_published_pkg_cache_download_failure() {
        let (locked_package, manifest_package) = locked_published_package(None);
        let mut client = MockClient::new();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };

        // This is a trick for a known storepath
        resp.items
            .insert(locked_package.outputs["out"].clone(), vec![
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
        client.push_store_info_response(resp);

        let buildenv = buildenv_instance();
        let result = buildenv.realise_nixpkgs(
            &client,
            &manifest_package,
            &locked_package,
            &Default::default(),
        );
        assert!(matches!(
            result,
            Err(BuildEnvError::BuildPublishedPackage(_))
        ));
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

    use indoc::formatdoc;
    use tempfile::TempDir;
    use test_helpers::buildenv_instance;

    use super::*;
    use crate::models::manifest::typed::PackageDescriptorFlake;
    use crate::providers::flake_installable_locker::{InstallableLocker, InstallableLockerImpl};

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
        let buildenv = buildenv_instance();

        assert!(
            !buildenv
                .check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be invalid before building"
        );

        let result = buildenv.realise_flakes(&locked_package, &Default::default());
        assert!(
            result.is_ok(),
            "failed to build flake: {}",
            result.unwrap_err()
        );
        assert!(
            buildenv
                .check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap()
        );
    }

    /// Realising a flake should fail if the output is not valid and cannot be built.
    #[test]
    fn flake_build_failure() {
        let locked_package = MockedLockedPackageFlake::builder()
            .succeed_build(false)
            .build();
        let buildenv = buildenv_instance();
        let result = buildenv.realise_flakes(&locked_package, &Default::default());
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

        let buildenv = buildenv_instance();
        assert!(
            !buildenv
                .check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be invalid before building"
        );

        let result = buildenv.realise_flakes(&locked_package, &Default::default());
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

        let buildenv = buildenv_instance();
        assert!(
            buildenv
                .check_store_path(locked_package.locked_installable.outputs.values())
                .unwrap(),
            "store path should be valid before building"
        );

        let result = buildenv.realise_flakes(&locked_package, &Default::default());
        assert!(result.is_ok(), "failed to skip building flake");
    }
}

#[cfg(test)]
mod realise_store_path_tests {
    use test_helpers::buildenv_instance;

    use super::*;
    use crate::models::manifest::typed::DEFAULT_PRIORITY;
    use crate::providers::auth::Auth;

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
        let buildenv = buildenv_instance();
        let locked = mock_store_path(true);

        // show that the store path is valid
        assert!(buildenv.check_store_path([&locked.store_path]).unwrap());
        let span = info_span!("dummy");

        BuildEnvNix::<PathBuf, Auth>::realise_single_store_path(
            &locked,
            buildenv.gc_root_base_path.path(),
            &Default::default(),
            span,
        )
        .expect("an existing store path should realise");
    }

    #[test]
    fn store_path_build_failure_if_invalid() {
        let buildenv = buildenv_instance();
        let locked = mock_store_path(false);

        // show that the store path is invalid
        assert!(!buildenv.check_store_path([&locked.store_path]).unwrap());
        let span = info_span!("dummy");

        let result = BuildEnvNix::<PathBuf, Auth>::realise_single_store_path(
            &locked,
            buildenv.gc_root_base_path.path(),
            &Default::default(),
            span,
        )
        .expect_err("invalid store path should fail to realise");
        assert!(matches!(result, BuildEnvError::Realise2 { .. }));
    }
}

#[cfg(test)]
mod buildenv_tests {
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;

    use test_helpers::buildenv_instance;

    use super::*;
    use crate::providers::catalog::{GENERATED_DATA, MANUALLY_GENERATED, MockClient};

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
        buildenv.build(&client, &lockfile_path, None).unwrap()
    });

    #[test]
    fn build_contains_binaries() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.runtime;
        assert!(runtime.join("bin/hello").exists());
        assert!(runtime.join("bin/hello").is_executable_file());

        let develop = result.develop.as_ref();
        assert!(develop.join("bin/hello").exists());
        assert!(develop.join("bin/hello").is_executable_file());
    }

    #[test]
    fn build_contains_activate_files() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.runtime;
        assert!(runtime.join("activate.d/start.bash").exists());
        assert!(runtime.join("activate.d/zsh").exists());
        assert!(runtime.join("etc/profile.d").is_dir());

        let develop = &result.develop;
        assert!(develop.join("activate.d/start.bash").exists());
        assert!(develop.join("activate.d/zsh").exists());
        assert!(develop.join("etc/profile.d").is_dir());
    }

    #[test]
    fn build_contains_lockfile() {
        let result = &*BUILDENV_RESULT_SIMPLE_PACKAGE;
        let runtime = &result.runtime;
        assert!(runtime.join("manifest.lock").exists());

        let develop = &result.develop;
        assert!(develop.join("manifest.lock").exists());
    }
    #[test]
    fn build_contains_build_script_and_output() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/build-noop/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();
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
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = &result.runtime;
        assert!(runtime.join("activate.d/hook-on-activate").exists());

        let develop = &result.develop;
        assert!(develop.join("activate.d/hook-on-activate").exists());
    }

    #[test]
    fn build_contains_profile_scripts() {
        let buildenv = buildenv_instance();
        let lockfile_path = GENERATED_DATA.join("envs/kitchen_sink/manifest.lock");
        let client = MockClient::new();
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        for output in [&result.runtime, &result.develop] {
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

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();

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
        let result = buildenv.build(&client, &lockfile_path, None);
        let err = result.expect_err("conflicting packages should fail to build");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        let expected =
            "> ❌ ERROR: 'vim' conflicts with 'vim-full'. Both packages provide the file 'bin/ex'";

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
        let result = buildenv.build(&client, &lockfile_path, None);
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
        let result = buildenv.build(&client, &lockfile_path, None);
        assert!(
            result.is_ok(),
            "environment should render succesfully: {}",
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
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();

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
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();
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
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();
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
        let result = buildenv.build(&client, &lockfile_path, None);
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
        let result = buildenv.build(&client, &lockfile_path, None);
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
    fn v2_manifest_default_outputs_includes_man() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with no outputs specified (should use outputs_to_install)
        let lockfile_path = GENERATED_DATA.join("envs/bash_v2_default/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.runtime.as_ref();
        let develop = outputs.develop.as_ref();

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
    fn v2_manifest_outputs_all_includes_info() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with outputs = "all"
        let lockfile_path = GENERATED_DATA.join("envs/bash_v2_all/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.runtime.as_ref();
        let develop = outputs.develop.as_ref();

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
    fn v2_manifest_outputs_out_only_excludes_others() {
        let buildenv = buildenv_instance();
        let client = MockClient::new();

        // Get a v2 lockfile with outputs = ["out"]
        let lockfile_path = GENERATED_DATA.join("envs/bash_v2_out/manifest.lock");

        let result = buildenv.build(&client, &lockfile_path, None);
        assert!(
            result.is_ok(),
            "environment should build successfully: {}",
            result.as_ref().unwrap_err()
        );

        let outputs = result.unwrap();
        let runtime = outputs.runtime.as_ref();
        let develop = outputs.develop.as_ref();

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
