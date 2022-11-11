use derive_more::Constructor;

use runix::{BuildArgs, FlakeArgs, InputOverride, Installable, NixApi};
use thiserror::Error;

use crate::{
    flox::{Flox, FloxNixApi},
    prelude::Stability,
};

#[derive(Constructor)]
pub struct Package<'flox> {
    flox: &'flox Flox,
    installable: Installable,
    stability: Stability,
}

#[derive(Error, Debug)]
pub enum PackageBuildError<Nix: NixApi> {
    #[error("Error getting Nix instance")]
    NixInstance(()),
    #[error("Error getting flake args")]
    FlakeArgs(()),
    #[error("Error running nix: {0}")]
    NixRun(<Nix as NixApi>::BuildError),
}
impl Package<'_> {
    fn flake_args(&self) -> Result<FlakeArgs, ()> {
        Ok(FlakeArgs {
            override_inputs: vec![InputOverride {
                from: "floxpkgs/nixpkgs/nixpkgs".into(),
                to: format!("flake:nixpkgs-{}", self.stability),
            }],
        })
    }

    /// flox build
    /// runs `nix build <installable>`
    pub async fn build<Nix: FloxNixApi + NixApi>(&self) -> Result<(), PackageBuildError<Nix>> {
        let nix = self.flox.nix::<Nix>();

        let command_args = BuildArgs {
            flake_args: self.flake_args().map_err(PackageBuildError::FlakeArgs)?,
            installables: [self.installable.clone()].into(),
            ..Default::default()
        };

        nix.build(command_args)
            .await
            .map_err(PackageBuildError::NixRun)?;
        Ok(())
    }
}
