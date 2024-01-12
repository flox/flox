use std::fs;
use std::path::{Path, PathBuf};

use log::debug;
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
    ManagedPointer,
    UpdateResult,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
};
use crate::flox::{EnvironmentOwner, EnvironmentRef, Flox};
use crate::models::container_builder::ContainerBuilder;
use crate::models::environment_ref::EnvironmentName;
use crate::models::floxmetav2::{FloxmetaV2, FloxmetaV2Error};
use crate::models::lockfile::LockedManifest;
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

    #[error("could not create temporary environment")]
    CreateTempDotFlox(#[source] std::io::Error),
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
        pointer: ManagedPointer,
    ) -> Result<Self, RemoteEnvironmentError> {
        let floxmeta = match FloxmetaV2::open(flox, &pointer) {
            Ok(floxmeta) => floxmeta,
            Err(FloxmetaV2Error::NotFound(_)) => {
                debug!("cloning floxmeta for {}", pointer.owner);
                FloxmetaV2::clone(flox, &pointer)
                    .map_err(RemoteEnvironmentError::GetLatestVersion)?
            },
            Err(e) => Err(RemoteEnvironmentError::GetLatestVersion(e))?,
        };

        let path = path.as_ref().join(DOT_FLOX);
        fs::create_dir_all(&path).map_err(RemoteEnvironmentError::CreateTempDotFlox)?;

        let dot_flox_path =
            CanonicalPath::new(path).map_err(RemoteEnvironmentError::InvalidTempPath)?;

        let pointer_content = serde_json::to_string_pretty(&pointer).unwrap();
        fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        )
        .unwrap();

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
    pub fn new(flox: &Flox, pointer: ManagedPointer) -> Result<Self, RemoteEnvironmentError> {
        let path = tempfile::tempdir_in(&flox.temp_dir).unwrap().into_path();

        Self::new_in(flox, path, pointer)
    }

    pub fn owner(&self) -> &EnvironmentOwner {
        self.inner.owner()
    }

    pub fn env_ref(&self) -> EnvironmentRef {
        EnvironmentRef::new_from_parts(self.owner().clone(), self.name())
    }

    pub fn pointer(&self) -> &ManagedPointer {
        self.inner.pointer()
    }
}

impl Environment for RemoteEnvironment {
    /// Build the environment and create a result link as gc-root
    fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        self.inner.build(flox)
    }

    /// Lock the environment and return the lockfile contents
    fn lock(&mut self, flox: &Flox) -> Result<LockedManifest, EnvironmentError2> {
        self.inner.lock(flox)
    }

    fn build_container(&mut self, flox: &Flox) -> Result<ContainerBuilder, EnvironmentError2> {
        self.inner.build_container(flox)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        let result = self.inner.install(packages, flox)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        // TODO: clean up git branch for temporary environment
        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2> {
        let result = self.inner.uninstall(packages, flox)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError2> {
        let result = self.inner.edit(flox, contents)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Atomically update this environment's inputs
    fn update(
        &mut self,
        flox: &Flox,
        inputs: Vec<String>,
    ) -> Result<UpdateResult, EnvironmentError2> {
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
        groups_or_iids: &[String],
    ) -> Result<UpgradeResult, EnvironmentError2> {
        let result = self.inner.upgrade(flox, groups_or_iids)?;
        self.inner
            .push(false)
            .map_err(RemoteEnvironmentError::UpdateUpstream)?;
        Ok(result)
    }

    /// Extract the current content of the manifest
    fn manifest_content(&self, flox: &Flox) -> Result<String, EnvironmentError2> {
        self.inner.manifest_content(flox)
    }

    fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        self.inner.activation_path(flox)
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
