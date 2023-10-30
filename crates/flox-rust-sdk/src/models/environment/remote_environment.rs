use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use runix::command_line::NixCommandLine;
use runix::installable::FlakeAttribute;

use super::{Environment, EnvironmentError2, InstallationAttempt};
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner, EnvironmentRef};

#[derive(Debug)]
pub struct RemoteEnvironment;

#[async_trait]
impl Environment for RemoteEnvironment {
    /// Build the environment and create a result link as gc-root
    #[allow(unused)]
    async fn build(
        &mut self,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<(), EnvironmentError2> {
        todo!()
    }

    /// Install packages to the environment atomically
    #[allow(unused)]
    async fn install(
        &mut self,
        packages: Vec<String>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        todo!()
    }

    /// Uninstall packages from the environment atomically
    #[allow(unused)]
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<String, EnvironmentError2> {
        todo!()
    }

    /// Atomically edit this environment, ensuring that it still builds
    #[allow(unused)]
    async fn edit(
        &mut self,
        nix: &NixCommandLine,
        system: System,
        contents: String,
    ) -> Result<(), EnvironmentError2> {
        todo!()
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
    fn manifest_content(&self) -> Result<String, EnvironmentError2> {
        todo!()
    }

    /// Return the [EnvironmentRef] for the environment for identification
    #[allow(unused)]
    fn environment_ref(&self) -> EnvironmentRef {
        todo!()
    }

    #[allow(unused)]
    fn flake_attribute(&self, system: System) -> FlakeAttribute {
        todo!()
    }

    /// Returns the environment owner
    #[allow(unused)]
    fn owner(&self) -> Option<EnvironmentOwner> {
        todo!()
    }

    /// Returns the environment name
    #[allow(unused)]
    fn name(&self) -> EnvironmentName {
        todo!()
    }

    /// Delete the Environment
    #[allow(unused)]
    fn delete(self) -> Result<(), EnvironmentError2> {
        todo!()
    }
}
