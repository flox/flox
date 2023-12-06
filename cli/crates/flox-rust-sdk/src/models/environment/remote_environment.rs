use std::path::PathBuf;

use async_trait::async_trait;
use flox_types::catalog::{EnvCatalog, System};
use runix::command_line::NixCommandLine;

use super::{EditResult, Environment, EnvironmentError2, InstallationAttempt};
use crate::flox::Flox;
use crate::models::environment_ref::EnvironmentName;

#[derive(Debug)]
pub struct RemoteEnvironment;

#[async_trait]
impl Environment for RemoteEnvironment {
    /// Build the environment and create a result link as gc-root
    #[allow(unused)]
    async fn build(&mut self, flox: &Flox) -> Result<(), EnvironmentError2> {
        todo!()
    }

    /// Install packages to the environment atomically
    #[allow(unused)]
    async fn install(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<InstallationAttempt, EnvironmentError2> {
        todo!()
    }

    /// Uninstall packages from the environment atomically
    #[allow(unused)]
    async fn uninstall(
        &mut self,
        packages: Vec<String>,
        flox: &Flox,
    ) -> Result<String, EnvironmentError2> {
        todo!()
    }

    /// Atomically edit this environment, ensuring that it still builds
    #[allow(unused)]
    async fn edit(
        &mut self,
        flox: &Flox,
        contents: String,
    ) -> Result<EditResult, EnvironmentError2> {
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

    #[allow(unused)]
    async fn activation_path(&mut self, flox: &Flox) -> Result<PathBuf, EnvironmentError2> {
        todo!()
    }

    #[allow(unused)]
    fn parent_path(&self) -> Result<PathBuf, EnvironmentError2> {
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
