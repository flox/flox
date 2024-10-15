use crate::flox::Flox;
use std::path::Path;

pub struct PublishProvider {
    repo_dir: Path,
    package_name: String,
}

impl PublishProvider {
    pub fn new(repo_dir: Path, package: &str) -> Self {
        Self { repo_dir: repo_dir, package_name: package.to_string() }
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
    pub fn prepublish_check(repo_dir: &Path, package_name: &str)
    {
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
    pub fn prepublish_build<B: ManifestBuilder>(builder: &B, _sandbox: &Path)
    {
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
    pub fn gather_build_info(_sandbox: &Path)
    {
        // Access lockfile of _sandbox to get base_catalog_{locked_url, rev, rev_count, rev_date}
        // populate the following:
        //      how do we get the drv_path, outputs(and paths), system from this?
    }

    /// Uses client catalog to...
    /// ... check access to the catalog
    /// ... check presence of and create the package if needed
    /// ... publish the build info
    pub fn publish_to_catalog()
    {
        // We should have all the info now.

    }

}