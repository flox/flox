use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::OsStr;
use std::hash::Hash;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use flox_core::canonical_path::CanonicalPath;
use pollster::FutureExt as _;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, info_span, instrument};

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
use crate::models::nix_plugins::NIX_PLUGINS;
use crate::providers::catalog::CatalogClientError;
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
    #[error("Failed to constructed environment: {0}")]
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

    /// An error that occurred while deserializing the output of the `nix build` command.
    #[error("Failed to deserialize 'nix build' output:\n{output}\nError: {err}")]
    ReadOutputs {
        output: String,
        err: serde_json::Error,
    },
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

    fn link(
        &self,
        store_path: impl AsRef<Path>,
        destination: &BuiltStorePath,
    ) -> Result<(), BuildEnvError>;
}

pub struct BuildEnvNix;

impl BuildEnvNix {
    fn base_command(&self) -> Command {
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

        check_store_paths(all_paths)
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
        for package in lockfile.packages.iter() {
            if package.system() != system {
                continue;
            }
            match package {
                LockedPackage::Catalog(locked) => {
                    self.realise_nixpkgs(client, locked, pre_checked_store_paths)?
                },
                LockedPackage::Flake(locked) => {
                    self.realise_flakes(locked, pre_checked_store_paths)?
                },
                LockedPackage::StorePath(locked) => {
                    self.realise_store_path(locked, pre_checked_store_paths)?
                },
            }
        }
        Ok(())
    }

    /// Try to substitute a published package by copying it from an associated store.
    ///
    /// Query the associated store(s) that contain the package from the catalog.
    /// Then attempt to download the package closure from each store in order,
    /// until successful.
    /// Returns `true` if all outputs were found and downloaded, `false` otherwise.
    fn try_substitute_published_pkg(
        &self,
        client: &impl ClientTrait,
        locked: &LockedPackageCatalog,
    ) -> Result<bool, BuildEnvError> {
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
        let mut all_found = true;
        'path_loop: for (path, locations) in store_locations.iter() {
            // If there are no locations
            for location in locations {
                // nix copy
                let mut copy_command = nix_base_command();
                copy_command
                    .arg("copy")
                    .arg("--from")
                    .arg(&location.url)
                    .arg(path);
                let output = copy_command
                    .output()
                    .map_err(|e| BuildEnvError::CacheError(e.to_string()))?;
                if !output.status.success() {
                    // If we failed, log the error and try the next location.
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    debug!(%path, %location.url, %stderr, "Failed to copy package from store");
                } else {
                    // If we suceeded, then we can continue with the nex path
                    debug!(%path, %location.url, "Succesfully copied package from store");
                    continue 'path_loop;
                }
            }
            // If we get here, we could not download the current path from anywhere
            debug!(%path, "Failed to copy path from any provided location");
            all_found = false;
        }

        Ok(all_found)
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
    ///
    /// IMPORTANT/TODO: As custom catalogs, with non-nixpkgs packages are in development,
    /// this function is currently assumes that the package is from the nixpkgs base-catalog.
    /// Currently the type is distinguished by the [LockedPackageCatalog::locked_url].
    /// If this does not indicate a nixpkgs package, the function will currently panic!
    fn realise_nixpkgs(
        &self,
        client: &impl ClientTrait,
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
            span.in_scope(|| self.check_store_path_with_substituters(locked.outputs.values()))?
        };

        // If all store paths are valid after substitution, we can return early.
        if all_valid_after_build_or_substitution {
            return Ok(());
        }

        let _span = info_span!(
            "build from catalog",
            progress = format!("Building '{}' from source", locked.attr_path)
        )
        .entered();

        let installable = {
            let mut locked_url = locked.locked_url.to_string();

            if let Some(revision_suffix) = locked_url.strip_prefix(NIXPKGS_CATALOG_URL_PREFIX) {
                locked_url = format!("{FLOX_NIXPKGS_PROXY_FLAKE_REF_BASE}/{revision_suffix}");
            } else {
                debug!(?locked.attr_path, "Trying to substitute published package");
                let all_found = self.try_substitute_published_pkg(client, locked)?;
                // We asked for all the outputs for the package, got store info for
                // each, and were able to substitute them all.  If so, then we're done here.
                if all_found {
                    return Ok(());
                };
                todo!("Building published packages is not yet supported");
            }

            // build all out paths
            let attrpath = format!("legacyPackages.{}.{}^*", locked.system, locked.attr_path);

            format!("{}#{}", locked_url, attrpath)
        };

        let mut nix_build_command = self.base_command();

        nix_build_command.args(["--option", "extra-plugin-files", &*NIX_PLUGINS]);

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--no-link");
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

        let mut nix_build_command = self.base_command();

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
    #[instrument(skip(self), fields(progress = format!("Realising store path for '{}'", locked.install_id)))]
    fn realise_store_path(
        &self,
        locked: &LockedPackageStorePath,
        pre_checked_store_paths: &CheckedStorePaths,
    ) -> Result<(), BuildEnvError> {
        let valid = pre_checked_store_paths
            .valid(&locked.store_path)
            .unwrap_or_default()
            || self.check_store_path([&locked.store_path])?;

        if !valid {
            return Err(BuildEnvError::Realise2 {
                install_id: locked.install_id.clone(),
                message: format!("'{}' is not available", locked.store_path),
            });
        }
        Ok(())
    }

    /// Check if the given store paths _exists_ on the filesystem,
    /// or in the configured nix store.
    /// Substitute store paths if necessary and possible.
    ///
    /// If the store paths do not exist,
    /// the function will fall back to querying the nix store for the store paths.
    /// Formerly, this function checked the store paths with `nix build` immediately,
    /// which would also ensure the integrity of the references of the store paths.
    /// However, the runtime profile of the `nix build` command
    /// has significant overhead for large environments.
    /// 50ms to 100ms per package in an environment of 50 packages,
    /// is very noticeable.
    /// To address this we replace the nix call with a number of `stat`
    /// calls for the paths that are checked, with the optimistic assumption
    /// that if a path exists, it and its references are valid.
    /// If they are not, we fall back to the nix call,
    /// which checking against alternative stores and substitution from binary caches.
    fn check_store_path_with_substituters(
        &self,
        paths: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<bool, BuildEnvError> {
        let mut cmd = self.base_command();
        cmd.arg("build");
        cmd.arg("--no-link");
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
        let mut nix_build_command = self.base_command();
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
        let [build_env_result]: [BuildEnvResultRaw; 1] = serde_json::from_slice(&output.stdout)
            .map_err(|err| BuildEnvError::ReadOutputs {
                output: String::from_utf8_lossy(&output.stdout).to_string(),
                err,
            })?;
        let outputs = build_env_result.outputs;
        Ok(outputs)
    }
}

impl BuildEnv for BuildEnvNix {
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
        if let Some(ref systems) = lockfile.manifest.options.systems {
            if !systems.contains(&env!("NIX_TARGET_SYSTEM").to_string()) {
                return Err(BuildEnvError::LockfileIncompatible {
                    systems: systems.clone(),
                });
            }
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

    fn link(
        &self,
        destination: impl AsRef<Path>,
        store_path: &BuiltStorePath,
    ) -> Result<(), BuildEnvError> {
        let mut nix_build_command = self.base_command();

        nix_build_command.arg("build").arg(store_path.as_ref());
        nix_build_command
            .arg("--out-link")
            .arg(destination.as_ref());

        // avoid trying to substitute
        nix_build_command.arg("--offline");

        debug!(cmd=%nix_build_command.display(), "linking store path");

        let output = nix_build_command
            .output()
            .map_err(BuildEnvError::CallNixBuild)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(BuildEnvError::Link(stderr.to_string()));
        }

        Ok(())
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

    let mut child = command.spawn().map_err(BuildEnvError::CallNixBuild)?;
    let stdin = child.stdin.as_mut().unwrap();

    let paths = paths
        .into_iter()
        .map(|p| p.as_ref().to_string())
        .collect::<HashSet<_>>();

    for path in paths.iter() {
        stdin.write_all(path.as_bytes()).unwrap();
        stdin.write_all(b"\n").unwrap();
    }

    stdin.flush().unwrap();

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

#[cfg(test)]
mod realise_nixpkgs_tests {

    use super::*;
    use crate::models::lockfile;
    use crate::providers::catalog::{MockClient, StoreInfo, StoreInfoResponse, GENERATED_DATA};
    use crate::providers::nix::test_helpers::known_store_path;

    /// Read a single locked package for the current system from a mock lockfile.
    /// This is a helper function to avoid repetitive boilerplate in the tests.
    /// The lockfiles are generated by the `mk_data`, by using `flox lock-manifest`.
    fn locked_package_catalog_from_mock(mock_lockfile: impl AsRef<Path>) -> LockedPackageCatalog {
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
            });
        locked_package.expect("no locked package found")
    }

    fn locked_published_package(store_path: Option<&str>) -> LockedPackageCatalog {
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        // Set the attr_path to something that looks like a published package.
        locked_package.attr_path = "custom_catalog/hello".to_string();
        locked_package.locked_url =
            "github:super/custom/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string();

        // replace the store path with a known invalid one, to trigger an attempt to rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid";
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            store_path.unwrap_or(invalid_store_path).to_string(),
        );
        locked_package
    }

    /// When a package is not available in the store, it should be built from its derivation.
    /// This test sets a known invalid store path to trigger a rebuild of the 'hello' package.
    /// Since we're unable to provide unique store paths for each test run,
    /// this test is only indicative that we _actually_ build the package.
    #[test]
    fn nixpkgs_build_reproduce_if_invalid() {
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new(None::<String>).unwrap();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // Note: Packages from the catalog are always possibly present already
        // especially if they are built by a previous run of the test suite.
        // hence we can't check if they are invalid before building.

        let buildenv = BuildEnvNix;

        let result = buildenv.realise_nixpkgs(&client, &locked_package, &Default::default());
        assert!(result.is_ok());

        // Note: per the above this may be incidentally true
        assert!(buildenv.check_store_path([original_store_path]).unwrap());
    }

    /// When a package is available in the store, it should not be evaluated or built.
    /// This test sets the attribute path to a known bad value,
    /// to ensure that the build will fail if buildenv attempts to evaluate the package.
    #[test]
    fn nixpkgs_skip_eval_if_valid() {
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new(None::<String>).unwrap();

        // build the package to ensure it is in the store
        let buildenv = BuildEnvNix;
        buildenv
            .realise_nixpkgs(&client, &locked_package, &Default::default())
            .expect("'hello' package should build");

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();
        buildenv
            .realise_nixpkgs(&client, &locked_package, &Default::default())
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
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello/manifest.lock"));
        let client = MockClient::new(None::<String>).unwrap();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&client, &locked_package, &Default::default());
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
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/hello-unfree-lock.json"));
        let client = MockClient::new(None::<String>).unwrap();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&client, &locked_package, &Default::default());
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
        let mut locked_package =
            locked_package_catalog_from_mock(GENERATED_DATA.join("envs/tabula-lock.json"));
        let client = MockClient::new(None::<String>).unwrap();

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&client, &locked_package, &Default::default());
        assert!(result.is_ok(), "{}", result.unwrap_err());
    }

    #[test]
    fn nixpkgs_published_pkg_no_matching_response() {
        let locked_package = locked_published_package(None);
        let mut client = MockClient::new(None::<String>).unwrap();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };

        resp.items
            .insert(locked_package.outputs["out"].clone(), vec![StoreInfo {
                url: "https://example.com".to_string(),
            }]);
        client.push_store_info_response(resp);

        let buildenv = BuildEnvNix;
        let subst_resp = buildenv
            .try_substitute_published_pkg(&client, &locked_package)
            .unwrap();
        assert!(!subst_resp);
    }

    #[test]
    fn nixpkgs_published_pkg_no_cache_info() {
        let locked_package = locked_published_package(None);
        let mut client = MockClient::new(None::<String>).unwrap();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };
        let fake_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        resp.items.insert(fake_store_path.clone(), vec![]);
        client.push_store_info_response(resp);

        let buildenv = BuildEnvNix;
        let subst_resp = buildenv
            .try_substitute_published_pkg(&client, &locked_package)
            .unwrap();
        assert!(!subst_resp);
    }

    #[test]
    fn nixpkgs_published_pkg_cache_download_success() {
        let real_storepath = known_store_path();
        let real_storepath_str = real_storepath.to_string_lossy();
        let locked_package = locked_published_package(Some(&real_storepath_str));
        let mut client = MockClient::new(None::<String>).unwrap();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };

        // This is a trick for a known storepath
        resp.items.insert(real_storepath_str.to_string(), vec![
            // Put something invalid first, to test that we try all locations
            StoreInfo {
                url: "blasphemy*".to_string(),
            },
            StoreInfo {
                url: "daemon".to_string(),
            },
        ]);
        client.push_store_info_response(resp);

        let buildenv = BuildEnvNix;
        let subst_resp = buildenv
            .try_substitute_published_pkg(&client, &locked_package)
            .unwrap();
        assert!(subst_resp);
    }

    #[test]
    #[should_panic = "Building published packages is not yet supported"]
    fn nixpkgs_published_pkg_cache_download_failure() {
        let locked_package = locked_published_package(None);
        let mut client = MockClient::new(None::<String>).unwrap();
        let mut resp = StoreInfoResponse {
            items: std::collections::HashMap::new(),
        };

        // This is a trick for a known storepath
        resp.items
            .insert(locked_package.outputs["out"].clone(), vec![
                // Put something invalid first, to test that we try all locations
                StoreInfo {
                    url: "blasphemy*".to_string(),
                },
                StoreInfo {
                    url: "daemon".to_string(),
                },
            ]);
        client.push_store_info_response(resp);

        let buildenv = BuildEnvNix;
        let _result = buildenv.realise_nixpkgs(&client, &locked_package, &Default::default());
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
        let buildenv = BuildEnvNix;

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
        assert!(buildenv
            .check_store_path(locked_package.locked_installable.outputs.values())
            .unwrap());
    }

    /// Realising a flake should fail if the output is not valid and cannot be built.
    #[test]
    fn flake_build_failure() {
        let locked_package = MockedLockedPackageFlake::builder()
            .succeed_build(false)
            .build();
        let buildenv = BuildEnvNix;
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

        let buildenv = BuildEnvNix;
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

        let buildenv = BuildEnvNix;
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
    use super::*;
    use crate::models::manifest::typed::DEFAULT_PRIORITY;

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
        let buildenv = BuildEnvNix;
        let locked = mock_store_path(true);

        // show that the store path is valid
        assert!(buildenv.check_store_path([&locked.store_path]).unwrap());

        buildenv
            .realise_store_path(&locked, &Default::default())
            .expect("an existing store path should realise");
    }

    #[test]
    fn store_path_build_failure_if_invalid() {
        let buildenv = BuildEnvNix;
        let locked = mock_store_path(false);

        // show that the store path is invalid
        assert!(!buildenv.check_store_path([&locked.store_path]).unwrap());

        let result = buildenv
            .realise_store_path(&locked, &Default::default())
            .expect_err("invalid store path should fail to realise");
        assert!(matches!(result, BuildEnvError::Realise2 { .. }));
    }
}

#[cfg(test)]
mod buildenv_tests {
    use std::collections::HashSet;
    use std::os::unix::fs::PermissionsExt;

    use regex::Regex;

    use super::*;
    use crate::providers::catalog::{MockClient, GENERATED_DATA, MANUALLY_GENERATED};

    trait PathExt {
        fn is_executable_file(&self) -> bool;
    }

    impl PathExt for Path {
        fn is_executable_file(&self) -> bool {
            self.is_file() && self.metadata().unwrap().permissions().mode() & 0o111 != 0
        }
    }

    static BUILDENV_RESULT_SIMPLE_PACKAGE: LazyLock<BuildEnvOutputs> = LazyLock::new(|| {
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/hello/manifest.lock");
        let client = MockClient::new(None::<String>).unwrap();
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
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/build-noop/manifest.lock");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = result.runtime.as_ref();
        let develop = result.develop.as_ref();
        let build_hello = result.manifest_build_runtimes.get("build-hello").unwrap();

        assert!(runtime.join("package-builds.d/hello").exists());
        assert!(develop.join("package-builds.d/hello").exists());
        assert!(build_hello.join("package-builds.d/hello").exists());
    }

    #[test]
    fn build_on_activate_lockfile() {
        let buildenv = BuildEnvNix;
        let lockfile_path = MANUALLY_GENERATED.join("buildenv/lockfiles/on-activate/manifest.lock");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None).unwrap();

        let runtime = &result.runtime;
        assert!(runtime.join("activate.d/hook-on-activate").exists());

        let develop = &result.develop;
        assert!(develop.join("activate.d/hook-on-activate").exists());
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
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/vim-vim-full-conflict.json");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None);
        let err = result.expect_err("conflicting packages should fail to build");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        let output_matches = Regex::new("error: collision between .*-vim-.* and .*-vim-.*")
            .unwrap()
            .is_match(&output);

        assert!(
            output_matches,
            "expected output to contain a conflict message: {output}"
        );
    }

    #[test]
    fn resolves_conflicting_packages_with_priority() {
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/vim-vim-full-conflict-resolved.json");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None);
        assert!(
            result.is_ok(),
            "conflicting packages should be resolved by priority: {}",
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
        let buildenv = BuildEnvNix;
        let lockfile_path = MANUALLY_GENERATED.join("buildenv/lockfiles/vars_escape/manifest.lock");
        let client = MockClient::new(None::<String>).unwrap();
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
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-all-toplevel.json");
        let client = MockClient::new(None::<String>).unwrap();
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
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-only-hello.json");
        let client = MockClient::new(None::<String>).unwrap();
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
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-not-toplevel.json");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None);
        let err = result.expect_err("build should fail if non-toplevel packages are selected");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        assert!(output.contains("error: package 'vim' is not in 'toplevel' pkg-group"));
    }

    #[test]
    fn verify_build_closure_cannot_select_nonexistent_packages_in_runtime_packages_attribute() {
        let buildenv = BuildEnvNix;
        let lockfile_path = GENERATED_DATA.join("envs/build-runtime-packages-not-found.json");
        let client = MockClient::new(None::<String>).unwrap();
        let result = buildenv.build(&client, &lockfile_path, None);
        let err = result.expect_err("build should fail if nonexistent packages are selected");

        let BuildEnvError::Build(output) = err else {
            panic!("expected build to fail, got {}", err);
        };

        assert!(output
            .contains("error: package 'goodbye' not found in '[install]' section of manifest"));
    }
}
