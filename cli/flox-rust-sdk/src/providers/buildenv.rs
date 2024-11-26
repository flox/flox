use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::models::pkgdb::{call_pkgdb, CallPkgDbError, PkgDbError, PKGDB_BIN};
use crate::models::lockfile::{
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
};
use crate::utils::CommandExt;

pub static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("NIX_BIN")
        .unwrap_or_else(|_| env!("NIX_BIN").to_string())
        .into()
});

static BUILDENV_NIX: LazyLock<PathBuf> = LazyLock::new(|| {
    std::env::var("FLOX_BUILDENV_NIX")
        .unwrap_or_else(|_| env!("FLOX_BUILDENV_NIX").to_string())
        .into()
});

#[derive(Debug, Error)]
pub enum BuildEnvError {
    /// An error that occurred while realising the packages in the lockfile.
    /// Those are Nix errors pkgdb forwards to us as well as detection of
    /// disallowed packages.
    #[error("Failed to realise packages in lockfile")]
    Realise(#[source] PkgDbError),

    #[error("Failed to realise '{install_id}':\n{message}")]
    Realise2 { install_id: String, message: String },

    /// An error that occurred while composing the environment.
    /// I.e. `nix build` returned with a non-zero exit code.
    /// The error message is the stderr of the `nix build` command.
    // TODO: this requires to capture the stderr of the `nix build` command
    // or essentially "tee" it if we also want to forward the logs to the user.
    // At the moment the "interesting" logs
    // are emitted by the `pkgdb realise` portion of the build.
    // So in the interest of initial simplicity
    // we can defer forwarding the nix build logs and capture output with [Command::output].
    #[error("Failed to constructed environment: {0}")]
    Build(String),

    /// An error that occurred while linking a store path.
    #[error("Failed to link environment: {0}")]
    Link(String),

    /// An error that occurred while calling nix build.
    #[error("Failed to call 'nix build'")]
    CallNixBuild(#[source] std::io::Error),

    /// An error that occurred while deserializing the output of the `nix build` command.
    #[error("Failed to deserialize 'nix build' output")]
    ReadOutputs(#[source] serde_json::Error),
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
        let mut nix_build_command = Command::new(&*NIX_BIN);
        // Override nix config to use flake commands,
        // allow impure language features such as `builtins.storePath`,
        // and use the auto store (which is used by the preceding `pkgdb realise` command)
        // TODO: formalize this in a config file,
        // and potentially disable other user configs (allowing specific overrides)
        nix_build_command.args([
            "--option",
            "extra-experimental-features",
            "nix-command flakes",
        ]);
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
    ///    We set `--option pure-eval true` to avoid improve reproducibility
    ///    of the locked outputs, and allow the use of the eval-cache
    ///    to avoid costly re-evaluations.
    ///
    /// IMPORTANT/TODO: As custom catalogs, with non-nixpkgs packages are in development,
    /// this function is currently assumes that the package is from the nixpkgs base-catalog.
    /// Currently the type is distinguished by the [LockedPackageCatalog::locked_url].
    /// If this does not indicate a nixpkgs package, the function will currently panic!
    fn realise_nixpkgs(&self, locked: &LockedPackageCatalog) -> Result<(), BuildEnvError> {
        // check if all store paths are valid, if so, return without eval
        let all_valid = self.check_store_path(locked.outputs.values())?;

        if all_valid {
            return Ok(());
        }

        let mut nix_build_command = self.base_command();

        // for now assume the plugin is relative relative to the pkgdb binary
        // <pkgdb>
        // ├── bin
        // │   └── pkgdb
        // └── lib
        //     └── wrapped-nixpkgs-input.(so|dylib)
        {
            let pkgdb_lib_dir = Path::new(&*PKGDB_BIN)
                .ancestors()
                .nth(2)
                .expect("pkgdb is in '<store-path>/bin'")
                .join("lib")
                .join(format!("wrapped-nixpkgs-input{}", env!("libExt")));
            nix_build_command.args([
                "--option",
                "extra-plugin-files",
                &pkgdb_lib_dir.to_string_lossy(),
            ]);
        }

        let installable = {
            let mut locked_url = locked.locked_url.to_string();
            if let Some(revision_suffix) =
                locked_url.strip_prefix("https://github.com/flox/nixpkgs?rev=")
            {
                locked_url = format!("flox-nixpkgs:v0/flox/{revision_suffix}");
            } else {
                todo!(
                    "Building non-nixpkgs catalog packages is not yet supported.\n\
                    Pending implementation and decisions regarding representation in the lockfile"
                );
            }

            // build all out paths
            let attrpath = format!("legacyPackages.{}.{}^*", locked.system, locked.attr_path);

            format!("{}#{}", locked_url, attrpath)
        };

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--no-link");
        nix_build_command.arg(&installable);

        debug!(%installable, cmd=%nix_build_command.display(), "building catalog package:");

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
    fn realise_flakes(&self, locked: &LockedPackageFlake) -> Result<(), BuildEnvError> {
        // check if all store paths are valid, if so, return without eval
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

        println!("building flake package: {}", installable);

        nix_build_command.arg("build");
        nix_build_command.arg("--no-write-lock-file");
        nix_build_command.arg("--no-update-lock-file");
        nix_build_command.args(["--option", "pure-eval", "true"]);
        nix_build_command.arg("--no-link");
        nix_build_command.arg(&installable);

        debug!(%installable, cmd=%nix_build_command.display(), "building catalog package:");

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
    fn realise_store_path(&self, locked: &LockedPackageStorePath) -> Result<(), BuildEnvError> {
        let valid = self.check_store_path([&locked.store_path])?;
        if !valid {
            return Err(BuildEnvError::Realise2 {
                install_id: locked.install_id.clone(),
                message: format!("'{}' is not available", locked.store_path),
            });
        }
        Ok(())
    }

    /// Check if the given store paths are valid,
    /// i.e. if the store paths exist in the store.
    fn check_store_path(
        &self,
        paths: impl IntoIterator<Item = impl AsRef<OsStr>>,
    ) -> Result<bool, BuildEnvError> {
        let mut cmd = self.base_command();
        cmd.arg("path-info").args(paths);

        debug!(cmd=%cmd.display(), "checking store paths");

        let success = cmd
            .output()
            .map_err(BuildEnvError::CallNixBuild)?
            .status
            .success();

        Ok(success)
    }
}

impl BuildEnv for BuildEnvNix {
    fn build(
        &self,
        lockfile_path: &Path,
        service_config_path: Option<PathBuf>,
    ) -> Result<BuildEnvOutputs, BuildEnvError> {
        if env::var("_FLOX_TESTING_NO_BUILD").is_ok() {
            panic!("Can't build when _FLOX_TESTING_NO_BUILD is set");
        }
        // todo: use `stat` or `nix path-info` to filter out pre-existing store paths

        // Realise the packages in the lockfile
        //
        // Locking flakes may require using `ssh` for private flakes,
        // so don't clear PATH
        // We don't have tests for private flakes,
        // so make sure private flakes work after touching this.
        let mut pkgdb_realise_cmd = Command::new(Path::new(&*PKGDB_BIN));
        pkgdb_realise_cmd.arg("realise").arg(lockfile_path);

        debug!(cmd=%pkgdb_realise_cmd.display(), "realising packages");
        match call_pkgdb(pkgdb_realise_cmd, false) {
            Ok(_) => {},
            Err(CallPkgDbError::PkgDbError(err)) => return Err(BuildEnvError::Realise(err)),
            Err(err) => return Err(BuildEnvError::CallPkgDb(err)),
        }

        // build the environment
        let mut nix_build_command = self.base_command();

        nix_build_command.args(["build", "--no-link", "--offline", "--json"]);
        // build the derivation produced by evaluating the `buildenv.nix` file
        nix_build_command.arg("--file").arg(&*BUILDENV_NIX);
        // pass the lockfile path as an argument to the `buildenv.nix` file
        nix_build_command
            .arg("--argstr")
            .arg("manifestLock")
            .arg(lockfile_path);
        // pass the service config path as an argument to the `buildenv.nix` file
        // if it is provided
        if let Some(service_config_path) = &service_config_path {
            nix_build_command
                .arg("--argstr")
                .arg("serviceConfigYaml")
                .arg(service_config_path);
        }
        // ... use default values for the remaining arguments of the `buildenv.nix` function.

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

        let [build_env_result]: [BuildEnvResultRaw; 1] =
            serde_json::from_slice(&output.stdout).map_err(BuildEnvError::ReadOutputs)?;

        let outputs = build_env_result.outputs;
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
