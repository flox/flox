use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use indoc::indoc;
use serde::Deserialize;
use thiserror::Error;
use tracing::debug;

use crate::models::pkgdb::{call_pkgdb, CallPkgDbError, PkgDbError, PKGDB_BIN};
use crate::utils::CommandExt;

static NIX_BIN: LazyLock<PathBuf> = LazyLock::new(|| {
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

    /// Other errors arising from calling pkgdb and interpreting its output.
    #[error(transparent)]
    CallPkgDb(CallPkgDbError),

    /// An error that occurred while composing the environment.
    /// I.e. `nix build` returned with a non-zero exit code.
    /// The error message is the stderr of the `nix build` command.
    // TODO: this requires to capture the stderr of the `nix build` command
    // or essentially "tee" it if we also want to foward the logs to the user.
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

#[derive(Debug, Clone, Deserialize, Eq, PartialEq)]
pub struct BuildEnvOutputs {
    pub develop: BuiltStorePath,
    pub runtime: BuiltStorePath,
    // todo: add more build runtime outputs
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, derive_more::Deref, derive_more::AsRef)]
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

impl BuildEnv for BuildEnvNix {
    fn build(
        &self,
        lockfile_path: &Path,
        service_config_path: Option<PathBuf>,
    ) -> Result<BuildEnvOutputs, BuildEnvError> {
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

impl BuildEnvNix {
    fn base_command(&self) -> Command {
        let mut nix_build_command = Command::new(&*NIX_BIN);
        // Override nix config to use flake commands,
        // allow impure language features such as `builtins.storePath`,
        // and use the auto store (which is used by the preceding `pkgdb realise` command)
        // TODO: formalize this in a config file,
        // and potentially disable other user configs (allowing specific overrides)
        let nix_config = indoc! {"
            experimental-features = nix-command flakes
            pure-eval = false
            store = auto
        "};

        nix_build_command.env("NIX_CONFIG", nix_config);
        // we generally want to see more logs (we can always filter them out)
        nix_build_command.arg("--print-build-logs");

        nix_build_command
    }
}
