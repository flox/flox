use std::path::{Path, PathBuf};

use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use flox_types::version::Version;
use log::debug;
use runix::command_line::NixCommandLine;
use thiserror::Error;

use super::managed_environment::{remote_branch_name, ManagedEnvironment, ManagedEnvironmentError};
use super::{
    gcroots_dir,
    CanonicalPath,
    CanonicalizeError,
    EditResult,
    Environment,
    EnvironmentError2,
    InstallationAttempt,
};
use crate::flox::{EnvironmentOwner, EnvironmentRef, Flox};
use crate::models::environment_ref::EnvironmentName;
use crate::models::floxmetav2::{FloxmetaV2, FloxmetaV2Error};
use crate::models::manifest::PackageToInstall;
use crate::models::pkgdb::UpgradeResult;

#[derive(Debug, Error)]
pub enum RemoteEnvironmentError {
    #[error("open managed environment")]
    OpenManagedEnvironment(#[source] ManagedEnvironmentError),

    #[error("could not get latest version of environment")]
    GetLatestVersion(#[source] FloxmetaV2Error),

    #[error("could not update upstream environment")]
    UpdateUpstream(#[source] ManagedEnvironmentError),

    #[error("invalid temporary path for new environment")]
    InvalidTempPath(#[source] CanonicalizeError),
}

#[derive(Debug)]
pub struct RemoteEnvironment {
    inner: ManagedEnvironment,
}

impl RemoteEnvironment {
    /// Pull a remote environment into a provided (temporary) managed environment
    pub fn new_in(
        flox: &Flox,
        path: impl AsRef<Path>,
        env_ref: &EnvironmentRef,
    ) -> Result<Self, RemoteEnvironmentError> {
        let pointer = super::ManagedPointer {
            owner: env_ref.owner().clone(),
            name: env_ref.name().clone(),
            version: Version::<1>,
        };

        let floxmeta = match FloxmetaV2::open(flox, &pointer) {
            Ok(floxmeta) => floxmeta,
            Err(FloxmetaV2Error::NotFound(_)) => {
                debug!("cloning floxmeta for {}", pointer.owner);
                FloxmetaV2::clone(flox, &pointer)
                    .map_err(RemoteEnvironmentError::GetLatestVersion)?
            },
            Err(e) => Err(RemoteEnvironmentError::GetLatestVersion(e))?,
        };

        let dot_flox_path =
            CanonicalPath::new(path).map_err(RemoteEnvironmentError::InvalidTempPath)?;

        let out_link =
            gcroots_dir(flox, &pointer.owner).join(remote_branch_name(&flox.system, &pointer));

        let inner = ManagedEnvironment::open_with(floxmeta, flox, pointer, dot_flox_path, out_link)
            .map_err(RemoteEnvironmentError::OpenManagedEnvironment)?;

        Ok(Self { inner })
    }

    /// Pull a remote environment into a temporary managed environment
    ///
    /// Contrary to [`RemoteEnvironment::new_in`], this function will create a temporary directory
    /// in the flox temp directory which is cleared when the process ends.
    pub fn new(flox: &Flox, env_ref: &EnvironmentRef) -> Result<Self, RemoteEnvironmentError> {
        let path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();

        Self::new_in(flox, path, env_ref)
    }

    pub fn owner(&self) -> &EnvironmentOwner {
        self.inner.owner()
    }
}

#[async_trait]
impl Environment for RemoteEnvironment {
    /// Build the environment and create a result link as gc-root
    async fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        self.inner.build(flox).await
    }

    /// Install packages to the environment atomically
    async fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let result = self.inner.install(packages, flox).await?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        // TODO: clean up git branch for temporary environment
        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2> {
        let result = self.inner.uninstall(packages, flox).await?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    async fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, EnvironmentError2> {
        let result = self.inner.edit(flox, contents).await?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Atomically update this environment's inputs
    fn update(&mut self, flox: &Flox, inputs: Vec<String>) -> Result<String, EnvironmentError2> {
        let result = self.inner.update(flox, inputs)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: Vec<String>,
    ) -> Result<UpgradeResult, EnvironmentError2> {
        let result = self.inner.upgrade(flox, groups_or_iids)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    #[allow(unused)]
    async fn catalog(
        &self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<EnvCatalog, EnvironmentError2> {
        todo!()
    }

    /// Extract the current content of the manifest
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError2> {
        self.inner.manifest_content(flox)
    }

    async fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        self.inner.activation_path(flox).await
    }

    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2> {
        self.inner.parent_path()
    }

    /// Path to the environment definition file
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        self.inner.manifest_path(flox)
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        self.inner.lockfile_path(flox)
    }

    /// Returns the environment name
    fn name(&self) -> EnvironmentName {
        self.inner.name()
    }

    /// Delete the Environment
    ///
    /// The local version of this is rather ... useless.
    /// It just deletes the temporary directory.
    /// When extended to delete upstream environments, this will be more useful.
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError2> {
        self.inner.delete(flox)
    }
}
