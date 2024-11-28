use std::collections::HashMap;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use flox_core::canonical_path::CanonicalPath;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::data::System;
use crate::models::lockfile::{
    LockedPackage,
    LockedPackageCatalog,
    LockedPackageFlake,
    LockedPackageStorePath,
    Lockfile,
};
use crate::models::pkgdb::{PkgDbError, PKGDB_BIN};
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

    #[error(
        "Lockfile is not compatible with the current system\n\
        Supported systems: {0}", systems.join(", "))]
    LockfileIncompatible { systems: Vec<String> },

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

    /// Realise all store paths of packages that are installed to the environment,
    /// for the given system.
    /// This goes through all packages in the lockfile and realises them with
    /// the appropriate method for the package type.
    ///
    /// See the individual realisation functions for more details.
    // todo: return actual store paths built,
    // necessary when building manifest builds.
    fn realise_lockfile(&self, lockfile: &Lockfile, system: &System) -> Result<(), BuildEnvError> {
        for package in lockfile.packages.iter() {
            if package.system() != system {
                continue;
            }
            match package {
                LockedPackage::Catalog(locked) => self.realise_nixpkgs(locked)?,
                LockedPackage::Flake(locked) => self.realise_flakes(locked)?,
                LockedPackage::StorePath(locked) => self.realise_store_path(locked)?,
            }
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
        let [build_env_result]: [BuildEnvResultRaw; 1] =
            serde_json::from_slice(&output.stdout).map_err(BuildEnvError::ReadOutputs)?;
        let outputs = build_env_result.outputs;
        Ok(outputs)
    }
}

impl BuildEnv for BuildEnvNix {
    fn build(
        &self,
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
        self.realise_lockfile(&lockfile, &env!("NIX_TARGET_SYSTEM").to_string())?;

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

#[cfg(test)]
mod realise_nixpkgs_tests {

    use super::*;
    use crate::models::lockfile;
    use crate::providers::catalog::GENERATED_DATA;

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

    /// When a package is not available in the store, it should be built from its derivation.
    /// This test sets a known invalid store path to trigger a rebuild of the 'hello' package.
    /// Since we're unable to provide unique store paths for each test run,
    /// this test is only indicative that we _actually_ build the package.
    #[test]
    fn nixpkgs_build_reproduce_if_invalid() {
        let mut locked_package =
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

        let buildenv = BuildEnvNix;

        let result = buildenv.realise_nixpkgs(&locked_package);
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

        // build the package to ensure it is in the store
        let buildenv = BuildEnvNix;
        buildenv
            .realise_nixpkgs(&locked_package)
            .expect("'hello' package should build");

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();
        buildenv
            .realise_nixpkgs(&locked_package)
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

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        // replace the attr_path with one that is known to fail to evaluate
        locked_package.attr_path = "AAAAAASomeThingsFailToEvaluate".to_string();

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&locked_package);
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

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&locked_package);
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

        // replace the store path with a known invalid one, to trigger a rebuild
        let invalid_store_path = "/nix/store/xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx-invalid".to_string();
        let _original_store_path = std::mem::replace(
            locked_package.outputs.get_mut("out").unwrap(),
            invalid_store_path,
        );

        let buildenv = BuildEnvNix;
        let result = buildenv.realise_nixpkgs(&locked_package);
        assert!(result.is_ok(), "{}", result.unwrap_err());
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
