use std::path::PathBuf;

use thiserror::Error;

use super::build::ManifestBuilder;
use super::catalog::Client;
use super::git::GitCommandProvider;
use crate::flox::FloxhubToken;
use crate::models::environment::managed_environment::ManagedEnvironment;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::lockfile::LockedManifestCatalog;

pub enum PublishEnvironment {
    Path(PathEnvironment),
    Managed(ManagedEnvironment),
}

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
pub trait Publish {
    fn publish(
        &self,
        client: Client,
        builder: &dyn ManifestBuilder,
        environment: PublishEnvironment,
        package: String,
    ) -> Result<(), String>;
}

/// Ensures that the required metadata for publishing is consistent
#[allow(clippy::manual_non_exhaustive)]
pub struct CheckedEnvironmentMetadata {
    pub git_metadata: GitCommandProvider,
    pub environment_root_relative: PathBuf, // RelativePath
    pub lockfile: LockedManifestCatalog,

    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

impl From<&PathEnvironment> for CheckedEnvironmentMetadata {
    fn from(_value: &PathEnvironment) -> Self {
        // Ensures current commit is in remote
        // Gathers of the info needed for publish
        // ... locked_url, rev, rev_count, rev_date of the build repo from git
        // ... locked_url, rev, rev_count, rev_date of the base catalog from the
        //         lockfile of the build repo
        // ... version, description, name, attr_path from manifest
        // ... catalog from current git auth handle
        todo!()
    }
}
impl From<&ManagedEnvironment> for CheckedEnvironmentMetadata {
    fn from(_value: &ManagedEnvironment) -> Self {
        // This will come second, after the path environment is implemented
        todo!()
    }
}

// Represents the metadata for publish that comes from the build
pub struct PublishableBuild {
    // Define metadata coming from the build, e.g. outpaths
}

// Implement .From for various types of builders
impl From<&dyn ManifestBuilder> for PublishableBuild {
    fn from(_value: &dyn ManifestBuilder) -> Self {
        // Build (if needed) and collect the meta data needed to publish

        // Access a `flox build` builder
        // ... to clone into a sandbox and run
        // ... `flox activate; flox build;`
        // prepublish_build();
        // _sandbox = temp dir
        // Use GitCommandProvider to get remote and current rev of build_repo
        // Use GitCommandProvider to clone that remote/rev to _sandbox
        //   this will ensure it's in the remote
        // Load the manifest from the .flox of that repo to get the version/description/catalog
        // Confirm access to remote resources, login, etc, so as to not waste
        //   time if it's going to fail later or require user interaction to
        // continue the operation.

        // Obtains info from the build process (the sandboxed one above)
        // ... base_catalog_{locked_url, rev, rev_count, rev_date}
        // ... system, drv_path (??), outputs (??)
        // gather_build_info(_sandbox: &Path) {
        // Access lockfile of _sandbox to get base_catalog_{locked_url, rev, rev_count, rev_date}
        // populate the following:
        //      how do we get the drv_path, outputs(and paths), system from this?

        todo!()
    }
}

/// The `PublishProvider` is a concrete implementation of the `Publish` trait.
/// It is responsible for the actual implementation of the `Publish` trait,
/// i.e. the actual publishing of a package to a catalog.
///
/// The `PublishProvider` is a generic struct, parameterized by a `Builder` type,
/// to build packages before publishing.
pub struct PublishProvider<Builder> {
    /// Directory under which we will clone and build the
    _base_temp_dir: PathBuf,
    /// Token of the user to authenticate with the catalog
    _auth_token: FloxhubToken,
    /// Building of manifest packages
    _builder: Builder,
}

#[derive(Debug, Error)]
pub enum PublishError {
    #[error("This type of environment is not supported for publishing")]
    UnsupportedEnvironment,
}

/// (default) implementation of the `Publish` trait, i.e. the publish interface to publish.
impl<Builder> Publish for PublishProvider<&Builder>
where
    Builder: ManifestBuilder,
{
    fn publish(
        &self,
        _client: Client,
        builder: &dyn ManifestBuilder,
        environment: PublishEnvironment,
        _package: String,
    ) -> Result<(), String> {
        let env = match environment {
            PublishEnvironment::Managed(_env) => Err(PublishError::UnsupportedEnvironment),
            PublishEnvironment::Path(env) => Ok(env),
        };

        let _checked_meta: CheckedEnvironmentMetadata =
            CheckedEnvironmentMetadata::from(&env.unwrap());

        // How to access "Builder" from obove?
        let _build_meta: PublishableBuild = PublishableBuild::from(builder);

        // Uses client to...
        // ... check access to the catalog
        // ... check presence of and create the package if needed
        // ... publish the build info
        // publish_to_catalog()
        todo!()
    }
}
