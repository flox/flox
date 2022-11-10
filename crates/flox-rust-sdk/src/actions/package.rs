use anyhow::Result;
use derive_more::Constructor;
use runix::{
    arguments::{
        flake::{FlakeArgs, InputOverride},
        NixArgs,
    },
    command::Build,
};

use crate::{
    flox::{Flox, NixApiExt},
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
        Ok(FlakeArgs {
            override_inputs: vec![InputOverride {
                from: "floxpkgs/nixpkgs/nixpkgs".into(),
                to: format!("flake:nixpkgs-{}", self.stability),
            }],
        })
    }
}

impl<Nix: NixApiExt> Package<'_, Nix> {
    /// flox build
    /// runs `nix build <installable>`
    pub async fn build(&self) -> Result<()> {
        let nix = self.flox.nix()?;

        let command_args = Build {
            flake: (self.flake_args()?),
            installables: [self.installable.clone()].into(),
            ..Default::default()
        };

        let nix_args = NixArgs {
            config: Default::default(),
            common: Default::default(),
            command: (Box::new(command_args)),
        };

        nix.run(nix_args).await?;
        Ok(())
    }
}
