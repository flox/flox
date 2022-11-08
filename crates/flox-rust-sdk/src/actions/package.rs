use anyhow::Result;
use derive_more::Constructor;
use runix::{setting::OverrideInputs, FlakeArgs, FlakeArgsBuilder};

use crate::{
    flox::{Flox, NixApiExt},
    nix::command::BuildBuilder,
    nix::NixArgsBuilder,
    prelude::{Installable, Stability},
};

#[derive(Constructor)]
pub struct Package<'flox, Nix: NixApiExt> {
    flox: &'flox Flox<Nix>,
    installable: Installable,
    stability: Stability,
}

impl<Nix> Package<'_, Nix>
where
    Nix: NixApiExt,
{
    fn flake_args(&self) -> Result<FlakeArgs> {
        Ok(FlakeArgsBuilder::default()
            .override_inputs([OverrideInputs::new(
                "floxpkgs/nixpkgs/nixpkgs".into(),
                format!("flake:nixpkgs-{}", self.stability),
            )])
            .build()?)
    }
}

impl<Nix: NixApiExt> Package<'_, Nix> {
    /// flox build
    /// runs `nix build <installable>`
    pub async fn build(&self) -> Result<()> {
        let nix = self.flox.nix()?;

        let command_args = BuildBuilder::default()
            .flake(self.flake_args()?)
            .installables([self.installable.clone()])
            .build()?;

        let nix_args = NixArgsBuilder::default()
            .command(Box::new(command_args))
            .build()?;

        nix.run(nix_args).await?;
        Ok(())
    }
}
