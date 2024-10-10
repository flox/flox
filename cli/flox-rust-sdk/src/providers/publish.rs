use crate::flox::Flox;
use std::path::Path;

pub struct PublishProvider {
}

impl PublishProvider {
    pub fn new(repo_path: Path) -> Self {
        Self { }
    }

    /// initiates the publish process
    pub fn publish(&self) -> Result<(), String> {

        prepublish_check();

        prepublish_build();

        gather_build_info();

        publish_to_catalog();
    }

    /// Ensures current commit is in remote
    /// Gathers some of the info needed for publish
    /// ... locked_url, rev, rev_count, rev_date from git
    /// ... version, description, name from manifest
    /// ... attr_path (package name), catalog from ????
    pub fn prepublish_check();
    
    /// Access a `flox build` builder, probably another provider?
    /// ... to clone into a sandbox and run
    /// ... `flox activate; flox build;`
    pub fn prepublish_build();

    /// Obtains info from the build process (the sandboxed one above)
    /// ... base_catalog_{locked_url, rev, rev_count, rev_date}
    /// ... system, drv_path (??), outputs (??)
    pub fn gather_build_info();

    /// Uses client catalog to...
    /// ... check access to the catalog
    /// ... check presence of and create the package if needed
    /// ... publish the build info
    pub fn publish_to_catalog();




}