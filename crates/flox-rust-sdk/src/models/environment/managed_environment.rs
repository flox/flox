use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use runix::command_line::NixCommandLine;

use super::{Environment, EnvironmentError2};
use crate::models::environment_ref::{EnvironmentName, EnvironmentOwner, EnvironmentRef};
use crate::prelude::flox_package::FloxPackage;

#[derive(Debug)]
pub struct ManagedEnvironment;

#[async_trait]
impl Environment for ManagedEnvironment {
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
        packages: Vec<FloxPackage>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2> {
        todo!()
    }

    /// Uninstall packages from the environment atomically
    #[allow(unused)]
    async fn uninstall(
        &mut self,
        packages: Vec<FloxPackage>,
        nix: &NixCommandLine,
        system: System,
    ) -> Result<bool, EnvironmentError2> {
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

    /// Extract the current content of the manifest
    fn manifest_content(&self) -> Result<String, EnvironmentError2> {
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

    /// Return the [EnvironmentRef] for the environment for identification
    #[allow(unused)]
    fn environment_ref(&self) -> &EnvironmentRef {
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
