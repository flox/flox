use std::path::{Path, PathBuf};

use url::Url;

use super::build::ManifestBuilder;
use super::git::GitCommandProvider;
use crate::flox::FloxhubToken;
use crate::models::environment::managed_environment::ManagedEnvironment;
use crate::models::environment::path_environment::PathEnvironment;
use crate::models::environment::Environment;
use crate::models::lockfile::{self, LockedManifestCatalog};

/// The `Publish` trait describes the high level behavior of publishing a package to a catalog.
/// Authentication, upload, builds etc, are implementation details of the specific provider.
/// Modeling the behavior as a trait allows us to swap out the provider, e.g. a mock for testing.
pub trait Publish {
    /// // Note: `Environment` is a bit broad here, specifically,
    ///          since we dont support building of nonlocal environments.
    ///          My suggestion is usually to define an enum of exactly the supported environments,
    ///          despite the existring proliferation of `*Environment` types.
    ///
    ///              /// This coudl also be a more generally useful "LocalEnvironment" type
    ///              enum PublishEnvironment {
    ///                  Path(PathEnvironment),
    ///                  Managed(ManagedEnvironment),
    ///              }
    fn publish_ä(
        &self,
        environment: impl Environment,
        package: String,
        catalog: String,
    ) -> Result<(), String>;

    /// Alternative version of publish that takes more granular arguments, instead of depending on [Environment].
    /// The downside of this is that the argumetns are still directly derived from a single environemnt
    /// and must be consistent.
    /// As individual arguments, these invairants are not enforced.
    fn publish_ü(
        &self,
        git: GitCommandProvider,
        environment_root_relative: PathBuf, // RelativePath,
        // can be used to infer catalog page
        lockfile: LockedManifestCatalog,
        package: String,
        catalog: String,
    ) -> Result<(), String>;

    /// Alternative version of publish that takes more granular arguments, instead of depending directly on [Environment].
    /// Ensures the integrity of the environment metadata by requiring [CheckedEnvironmentMetadata].
    fn publish_ö(
        &self,
        environment: CheckedEnvironmentMetadata,
        package: String,
        catalog: String,
    ) -> Result<(), String>;

    /// Alternative version of publish that directly addresses an upstrean git repo, for publishing.
    /// Determining the upstream url from an [Environment] is a responsibility of the caller.
    fn publish_ë(
        &self,
        upstream_git_repo: Url,
        environment_root_relative: PathBuf,
        package: String,
        catalog: String,
    );
}

/// ensures that the required metadata for publishing is consistent
pub struct CheckedEnvironmentMetadata {
    pub git_metadata: GitCommandProvider,
    pub environment_root_relative: PathBuf, // RelativePath
    pub lockfile: LockedManifestCatalog,
    // This field isn't "pub", so no one outside this module can construct this struct. That helps
    // ensure that we can only make this struct as a result of doing the "right thing."
    _private: (),
}

impl From<&PathEnvironment> for CheckedEnvironmentMetadata {
    fn from(value: &PathEnvironment) -> Self {
        todo!()
    }
}
impl From<&ManagedEnvironment> for CheckedEnvironmentMetadata {
    fn from(value: &ManagedEnvironment) -> Self {
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
    base_temp_dir: PathBuf,
    /// Token of the user to authenticate with the catalog
    auth_token: FloxhubToken,
    /// Building of manifest packages
    builder: Builder,
}

/// (default) implementation of the `Publish` trait, i.e. the publis interface to publish.
// regarding best patterns  _behind_ the `Publish` inteface i'm less opinionated,
// I think typestates on the `PublishProvider` or a more functional typestate inspired
// approach like @zmitchell suggested could be a good fit.
impl<Builder> Publish for PublishProvider<&Builder> where Builder: ManifestBuilder {}

impl PublishProvider {
    pub fn new(repo_dir: Path, package: &str) -> Self {
        Self {
            repo_dir,
            package_name: package.to_string(),
        }
    }

    /// initiates the publish process
    pub fn publish(&self) -> Result<(), String> {
        // Run each phase, gathering info along the way until we get to the end
        // and can send it all to the catalog

        // Best pattern to use here?
        // - Store in structs in Self along the way?
        // - Type States (like the old publish?)
        // - Make each phase independant, taking arguments for all that's needed
        //    and track the ongoing state only here in publish()?
        // - Combination of Type States and making each phase independant may be the
        //    easiest to test?
        prepublish_check();

        prepublish_build();

        gather_build_info();

        publish_to_catalog();
    }

    /// Ensures current commit is in remote
    /// Gathers of the info needed for publish
    /// ... locked_url, rev, rev_count, rev_date of the build repo from git
    /// ... locked_url, rev, rev_count, rev_date of the base catalog from the
    ///         lockfile of the build repo
    /// ... version, description, name, attr_path from manifest
    /// ... catalog from ???? (cli argument? manifest?)
    pub fn prepublish_check(repo_dir: &Path, package_name: &str) {
        // _sandbox = temp dir
        // Use GitCommandProvider to get remote and current rev of build_repo
        // Use GitCommandProvider to clone that remote/rev to _sandbox
        // this will ensure it's in the remote
        // Load the manifest from the .flox of that repo to get the version/description/catalog
        // Confirm access to remote resources, login, etc, so as to not waste
        // time if it's going to fail later or require user interaction to
        // continue the operation.
    }

    /// Access a `flox build` builder, probably another provider?
    /// ... to clone into a sandbox and run
    /// ... `flox activate; flox build;`
    pub fn prepublish_build<B: ManifestBuilder>(builder: &B, _sandbox: &Path) {
        // We need to do enough to get the build info for the next step
        // ... equivalent to run `flox activate; flox build;`?

        // This will create a flox env in the _sandbox, right? so should we pass that back
        // and store it to use for the next step?

        // Use ManifestBuilder to perform the build (evaluation?) so we can get
        // the info we need.  Do we need to build? or is there something else?
        let build_output = builder.build(_sandbox, flox_env, &self.package_name);

        // the build_output looks to be the stdout of the build process.. how do
        // we access the drv_path and that stuff?
    }

    /// Obtains info from the build process (the sandboxed one above)
    /// ... base_catalog_{locked_url, rev, rev_count, rev_date}
    /// ... system, drv_path (??), outputs (??)
    pub fn gather_build_info(_sandbox: &Path) {
        // Access lockfile of _sandbox to get base_catalog_{locked_url, rev, rev_count, rev_date}
        // populate the following:
        //      how do we get the drv_path, outputs(and paths), system from this?
    }

    /// Uses client catalog to...
    /// ... check access to the catalog
    /// ... check presence of and create the package if needed
    /// ... publish the build info
    pub fn publish_to_catalog() {
        // We should have all the info now.
    }
}
