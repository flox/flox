use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;
use thiserror::Error;
use tracing::debug;

use super::core_environment::UpgradeResult;
use super::managed_environment::{remote_branch_name, ManagedEnvironment, ManagedEnvironmentError};
use super::{
    gcroots_dir,
    CanonicalPath,
    CanonicalizeError,
    EditResult,
    Environment,
    EnvironmentError,
    InstallationAttempt,
    ManagedPointer,
    RenderedEnvironmentLinks,
    UninstallationAttempt,
    DOT_FLOX,
    ENVIRONMENT_POINTER_FILENAME,
    GCROOTS_DIR_NAME,
};
use crate::flox::{EnvironmentOwner, EnvironmentRef, Flox};
use crate::models::environment::RenderedEnvironmentLink;
use crate::models::environment_ref::EnvironmentName;
use crate::models::floxmeta::{FloxMeta, FloxMetaError};
use crate::models::lockfile::Lockfile;
use crate::models::manifest::{Manifest, PackageToInstall};

const REMOTE_ENVIRONMENT_BASE_DIR: &str = "remote";

#[derive(Debug, Error)]
pub enum RemoteEnvironmentError {
    #[error("open managed environment")]
    OpenManagedEnvironment(#[source] ManagedEnvironmentError),

    #[error("could not create gc-root directory")]
    CreateGcRootDir(#[source] std::io::Error),

    #[error("could not get latest version of environment")]
    GetLatestVersion(#[source] FloxMetaError),

    #[error("could not reset managed environment")]
    ResetManagedEnvironment(#[source] ManagedEnvironmentError),

    #[error("could not update upstream environment")]
    UpdateUpstream(#[source] ManagedEnvironmentError),

    #[error("invalid temporary path for new environment")]
    InvalidTempPath(#[source] CanonicalizeError),

    #[error("could not create temporary environment")]
    CreateTempDotFlox(#[source] std::io::Error),

    /// the internal [ManagedEnvironment::activation_path] returned an invalid path
    #[error("could not determine location of new install prefix")]
    ReadInternalOutLink(#[source] std::io::Error),

    #[error("could not remove the existing install prefix")]
    DeleteOldOutLink(#[source] std::io::Error),

    #[error("could not set a new install prefix")]
    WriteNewOutlink(#[source] std::io::Error),
}

#[derive(Debug)]
pub struct RemoteEnvironment {
    inner: ManagedEnvironment,
    rendered_env_links: RenderedEnvironmentLinks,
}

impl RemoteEnvironment {
    /// Pull a remote environment into a provided (temporary) managed environment.
    /// Constructiing a [RemoteEnvironment] _does not_ create a gc-root
    /// or guarantee that the environment is valid.
    pub fn new_in(
        flox: &Flox,
        path: impl AsRef<Path>,
        pointer: ManagedPointer,
    ) -> Result<Self, RemoteEnvironmentError> {
        let floxmeta = match FloxMeta::open(flox, &pointer) {
            Ok(floxmeta) => floxmeta,
            Err(FloxMetaError::NotFound(_)) => {
                debug!("cloning floxmeta for {}", pointer.owner);
                FloxMeta::clone(flox, &pointer).map_err(RemoteEnvironmentError::GetLatestVersion)?
            },
            Err(e) => Err(RemoteEnvironmentError::GetLatestVersion(e))?,
        };

        let path = path.as_ref().join(DOT_FLOX);
        fs::create_dir_all(&path).map_err(RemoteEnvironmentError::CreateTempDotFlox)?;

        let dot_flox_path =
            CanonicalPath::new(&path).map_err(RemoteEnvironmentError::InvalidTempPath)?;

        let pointer_content = serde_json::to_string_pretty(&pointer).unwrap();
        fs::write(
            dot_flox_path.join(ENVIRONMENT_POINTER_FILENAME),
            pointer_content,
        )
        .unwrap();

        let inner_rendered_env_links = {
            let gcroots_dir = dot_flox_path.join(GCROOTS_DIR_NAME);

            // `.flox/run` used to be a link until flox versions 1.3.3!
            // If we find a symlink, we need to delete it to create a directory
            // with symlinked files in the following step.
            if gcroots_dir.exists() && gcroots_dir.is_symlink() {
                debug!(gcroot=?gcroots_dir, "removing symlink");
                fs::remove_file(&gcroots_dir).map_err(RemoteEnvironmentError::CreateGcRootDir)?;
            }

            if !gcroots_dir.exists() {
                std::fs::create_dir_all(&gcroots_dir)
                    .map_err(RemoteEnvironmentError::CreateGcRootDir)?;
            }

            let base_dir =
                CanonicalPath::new(gcroots_dir).expect("gcroots_dir is not a valid path");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                pointer.name.as_ref(),
                &flox.system,
            )
        };

        let mut inner = ManagedEnvironment::open_with(
            floxmeta,
            flox,
            pointer.clone(),
            dot_flox_path,
            inner_rendered_env_links,
        )
        .map_err(RemoteEnvironmentError::OpenManagedEnvironment)?;

        // (force) Pull latest changes of the environment from upstream.
        // remote environments stay in sync with upstream without providing a local staging state.
        inner
            .pull(flox, true)
            .map_err(RemoteEnvironmentError::ResetManagedEnvironment)?;

        let rendered_env_links = {
            let gcroots_dir = gcroots_dir(flox, &pointer.owner);
            if !gcroots_dir.exists() {
                std::fs::create_dir_all(&gcroots_dir)
                    .map_err(RemoteEnvironmentError::CreateGcRootDir)?;
            }
            let base_dir =
                CanonicalPath::new(gcroots_dir).expect("gcroots_dir is not a valid path");

            RenderedEnvironmentLinks::new_in_base_dir_with_name_and_system(
                &base_dir,
                remote_branch_name(&pointer),
                &flox.system,
            )
        };

        Ok(Self {
            inner,
            rendered_env_links,
        })
    }

    /// Pull a remote environment into a flox-provided managed environment
    /// in `<FLOX_CACHE_DIR>/remote/<owner>/<name>`
    ///
    /// This function provides the sensible default directory to [RemoteEnvironment::new_in].
    /// The directory will be created by [RemoteEnvironment::new_in].
    pub fn new(flox: &Flox, pointer: ManagedPointer) -> Result<Self, RemoteEnvironmentError> {
        let path = flox
            .cache_dir
            .join(REMOTE_ENVIRONMENT_BASE_DIR)
            .join(pointer.owner.as_ref())
            .join(pointer.name.as_ref());

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

    /// Update the out link to point to the current version of the environment
    ///
    /// The inner out link points to the latest version of the managed environment.
    /// This may be updated, but subsequently fail to push to the remote.
    /// In that case the remote environment should _not_ be changed.
    ///
    /// [RemoteEnvironment::update_out_link] updates the out link when the push succeeds.
    fn update_out_link(
        flox: &Flox,
        rendered_env_links: &RenderedEnvironmentLinks,
        inner: &mut ManagedEnvironment,
    ) -> Result<(), EnvironmentError> {
        let new_rendered_paths = inner.rendered_env_links(flox)?;

        fn update_link(
            old_link: &RenderedEnvironmentLink,
            new_link: &RenderedEnvironmentLink,
        ) -> Result<(), RemoteEnvironmentError> {
            let new_dev_link_path = new_link
                .read_link()
                .map_err(RemoteEnvironmentError::ReadInternalOutLink)?;

            debug!(gcroot=?old_link, to=?new_dev_link_path, "updating gcroot");

            if old_link.read_link().is_ok() {
                fs::remove_file(old_link).map_err(RemoteEnvironmentError::DeleteOldOutLink)?;
            }

            std::os::unix::fs::symlink(new_dev_link_path, old_link)
                .map_err(RemoteEnvironmentError::WriteNewOutlink)?;
            Ok(())
        }

        update_link(
            &rendered_env_links.development,
            &new_rendered_paths.development,
        )?;
        update_link(&rendered_env_links.runtime, &new_rendered_paths.runtime)?;

        Ok(())
    }
}

impl Environment for RemoteEnvironment {
    /// Return the lockfile content,
    /// or error if the lockfile doesn't exist.
    fn lockfile(&mut self, flox: &Flox) -> Result<Lockfile, EnvironmentError> {
        self.inner.lockfile(flox)
    }

    /// Install packages to the environment atomically
    fn install(
        &mut self,
        packages: &[PackageToInstall],
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError> {
        let result = self.inner.install(packages, flox)?;
        self.inner
            .push(flox, false)
            .map_err(|e| RemoteEnvironmentError::UpdateUpstream(e).into())
            .and_then(|_| Self::update_out_link(flox, &self.rendered_env_links, &mut self.inner))?;
        // TODO: clean up git branch for temporary environment
        Ok(result)
    }

    /// Uninstall packages from the environment atomically
    fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<UninstallationAttempt, EnvironmentError> {
        let result = self.inner.uninstall(packages, flox)?;
        self.inner
            .push(flox, false)
            .map_err(|e| RemoteEnvironmentError::UpdateUpstream(e).into())
            .and_then(|_| Self::update_out_link(flox, &self.rendered_env_links, &mut self.inner))?;

        Ok(result)
    }

    /// Atomically edit this environment, ensuring that it still builds
    fn edit(&mut self, flox: &Flox, contents: String) -> Result<EditResult, EnvironmentError> {
        let result = self.inner.edit(flox, contents)?;
        if result == EditResult::Unchanged {
            return Ok(result);
        }
        self.inner
            .push(flox, false)
            .map_err(|e| RemoteEnvironmentError::UpdateUpstream(e).into())
            .and_then(|_| Self::update_out_link(flox, &self.rendered_env_links, &mut self.inner))?;

        Ok(result)
    }

    /// Atomically upgrade packages in this environment
    fn upgrade(
        &mut self,
        flox: &Flox,
        groups_or_iids: &[&str],
    ) -> Result<UpgradeResult, EnvironmentError> {
        let result = self.inner.upgrade(flox, groups_or_iids)?;
        self.inner
            .push(flox, false)
            .map_err(|e| RemoteEnvironmentError::UpdateUpstream(e).into())
            .and_then(|_| Self::update_out_link(flox, &self.rendered_env_links, &mut self.inner))?;

        Ok(result)
    }

    /// Extract the current content of the manifest
    fn manifest_contents(&self, flox: &Flox) -> Result<String, EnvironmentError> {
        self.inner.manifest_contents(flox)
    }

    /// Return the deserialized manifest
    fn manifest(&self, flox: &Flox) -> Result<Manifest, EnvironmentError> {
        self.inner.manifest(flox)
    }

    fn rendered_env_links(
        &mut self,
        flox: &Flox,
    ) -> Result<RenderedEnvironmentLinks, EnvironmentError> {
        Self::update_out_link(flox, &self.rendered_env_links, &mut self.inner)?;
        Ok(self.rendered_env_links.clone())
    }

    fn build(
        &mut self,
        flox: &Flox,
    ) -> Result<crate::providers::buildenv::BuildEnvOutputs, EnvironmentError> {
        self.inner.build(flox)
    }

    /// Return a path that environment hooks should use to store transient data.
    ///
    /// Remote environments shouldn't have state of any kind, so this just
    /// returns a temporary directory.
    fn cache_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        let tempdir = TempDir::new().map_err(EnvironmentError::CreateTempDir)?;
        CanonicalPath::new(tempdir.into_path()).map_err(EnvironmentError::Canonicalize)
    }

    fn log_path(&self) -> Result<CanonicalPath, EnvironmentError> {
        self.inner.log_path()
    }

    fn project_path(&self) -> Result<PathBuf, EnvironmentError> {
        std::env::current_dir().map_err(EnvironmentError::GetCurrentDir)
    }

    fn parent_path(&self) -> Result<PathBuf, EnvironmentError> {
        self.inner.parent_path()
    }

    /// Path to the environment's .flox directory
    fn dot_flox_path(&self) -> CanonicalPath {
        self.inner.dot_flox_path()
    }

    /// Path to the environment definition file
    fn manifest_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        self.inner.manifest_path(flox)
    }

    /// Path to the lockfile. The path may not exist.
    fn lockfile_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
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
    fn delete(self, flox: &Flox) -> Result<(), EnvironmentError> {
        self.inner.delete(flox)
    }

    fn services_socket_path(&self, flox: &Flox) -> Result<PathBuf, EnvironmentError> {
        self.inner.services_socket_path(flox)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use super::*;
    use crate::flox::test_helpers::flox_instance_with_optional_floxhub;
    use crate::models::environment::managed_environment::test_helpers::mock_managed_environment_from_env_files;
    use crate::providers::catalog::GENERATED_DATA;

    #[test]
    fn migrate_remote_gcroot_link_to_dir() {
        let owner = "owner".parse().unwrap();
        let (flox, _temp_dir_handle) = flox_instance_with_optional_floxhub(Some(&owner));

        // Create a remote environment "owner/name"
        let environment = mock_managed_environment_from_env_files(
            &flox,
            GENERATED_DATA.join("envs").join("hello"),
            owner,
        );

        // Create a symlink, as it was done in older versions of flox prior to 1.3.4
        fs::remove_dir_all(environment.dot_flox_path().join(GCROOTS_DIR_NAME)).unwrap();
        symlink(
            "/dev/null",
            environment.dot_flox_path().join(GCROOTS_DIR_NAME),
        )
        .unwrap();

        assert!(environment
            .dot_flox_path()
            .join(GCROOTS_DIR_NAME)
            .is_symlink());

        // Create a remote environment with the existing managed environment as its backend
        let _ = RemoteEnvironment::new_in(
            &flox,
            environment.parent_path().unwrap(),
            environment.pointer().clone(),
        )
        .unwrap();

        // Once created, the symlink should be replaced with a directory
        assert!(environment.dot_flox_path().join(GCROOTS_DIR_NAME).is_dir())
    }
}
