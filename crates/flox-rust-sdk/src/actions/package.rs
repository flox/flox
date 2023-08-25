use derive_more::Constructor;
use flox_types::stability::Stability;
use runix::arguments::flake::FlakeArgs;
use runix::arguments::{BundleArgs, NixArgs};
use runix::command::{Build, Bundle, Develop, Run as RunCommand, Shell};
use runix::installable::{FlakeAttribute, Installable};
use runix::{NixBackend, Run};
use thiserror::Error;

use crate::flox::{Flox, FloxNixApi};

#[derive(Constructor)]
pub struct Package<'flox> {
    flox: &'flox Flox,
    flake_attribute: FlakeAttribute,
    stability: Option<Stability>,
    nix_arguments: Vec<String>,
}

/// Errors shared among package/development commands
#[derive(Error, Debug)]
pub enum PackageError {
    #[error("Error getting Nix instance")]
    NixInstance(()),
    #[error("Error getting flake args")]
    FlakeArgs(()),
}

#[derive(Error, Debug)]
pub enum PackageBuildError<Nix: NixBackend>
where
    Build: Run<Nix>,
{
    #[error(transparent)]
    Common(#[from] PackageError),
    #[error("Error running nix: {0}")]
    NixRun(<Build as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum PackageDevelopError<Nix: NixBackend>
where
    Develop: Run<Nix>,
{
    #[error(transparent)]
    Common(#[from] PackageError),
    #[error("Error running nix: {0}")]
    NixRun(<Develop as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum PackageRunError<Nix: NixBackend>
where
    RunCommand: Run<Nix>,
{
    #[error(transparent)]
    Common(#[from] PackageError),
    #[error("Error running nix: {0}")]
    NixRun(<RunCommand as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum PackageShellError<Nix: NixBackend>
where
    Shell: Run<Nix>,
{
    #[error(transparent)]
    Common(#[from] PackageError),
    #[error("Error running nix: {0}")]
    NixRun(<Shell as Run<Nix>>::Error),
}

#[derive(Error, Debug)]
pub enum PackageBundleError<Nix: NixBackend>
where
    Bundle: Run<Nix>,
{
    #[error(transparent)]
    Common(#[from] PackageError),
    #[error("Error running nix: {0}")]
    NixRun(<Bundle as Run<Nix>>::Error),
}

impl Package<'_> {
    fn flake_args(&self) -> Result<FlakeArgs, ()> {
        let override_inputs = self.stability.as_ref().map(Stability::as_override);
        Ok(FlakeArgs {
            override_inputs: Vec::from_iter(override_inputs),
            ..Default::default()
        })
    }

    /// flox build
    /// runs `nix build <installable>`
    pub async fn build<Nix: FloxNixApi>(&self) -> Result<(), PackageBuildError<Nix>>
    where
        Build: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>(self.nix_arguments.clone());

        let nix_args = NixArgs::default();

        let command = Build {
            flake: self.flake_args().map_err(PackageError::FlakeArgs)?,
            installables: [self.flake_attribute.clone().into()].into(),
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(PackageBuildError::NixRun)?;

        Ok(())
    }

    /// flox develop
    /// runs `nix develop <installable>`
    pub async fn develop<Nix: FloxNixApi>(&self) -> Result<(), PackageDevelopError<Nix>>
    where
        Develop: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>(self.nix_arguments.clone());

        let nix_args = NixArgs::default();

        let command = Develop {
            flake: self.flake_args().map_err(PackageError::FlakeArgs)?,
            installable: self.flake_attribute.clone().into(),
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(PackageDevelopError::NixRun)?;

        Ok(())
    }

    /// flox run
    /// runs `nix run <installable>`
    pub async fn run<Nix: FloxNixApi>(&self) -> Result<(), PackageRunError<Nix>>
    where
        RunCommand: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>(self.nix_arguments.clone());

        let nix_args = NixArgs::default();

        let command = RunCommand {
            flake: self.flake_args().map_err(PackageError::FlakeArgs)?,
            installable: self.flake_attribute.clone().into(),
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(PackageRunError::NixRun)?;

        Ok(())
    }

    /// flox shell
    /// runs `nix shell <installable>`
    pub async fn shell<Nix: FloxNixApi>(&self) -> Result<(), PackageShellError<Nix>>
    where
        Shell: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>(self.nix_arguments.clone());

        let nix_args = NixArgs::default();

        let command = Shell {
            flake: self.flake_args().map_err(PackageError::FlakeArgs)?,
            installables: [self.flake_attribute.clone().into()].into(),
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(PackageShellError::NixRun)?;

        Ok(())
    }

    /// flox bundle
    /// runs `nix bundle --bundler <installable> <installable>`
    pub async fn bundle<Nix: FloxNixApi>(
        &self,
        bundler: Installable,
    ) -> Result<(), PackageBundleError<Nix>>
    where
        Bundle: Run<Nix>,
    {
        let nix = self.flox.nix::<Nix>(self.nix_arguments.clone());

        let nix_args = NixArgs::default();

        let command = Bundle {
            flake: self.flake_args().map_err(PackageError::FlakeArgs)?,
            installable: self.flake_attribute.clone().into(),
            bundle_args: BundleArgs {
                bundler: Some(bundler.into()),
            },
            ..Default::default()
        };

        command
            .run(&nix, &nix_args)
            .await
            .map_err(PackageBundleError::NixRun)?;

        Ok(())
    }
}
