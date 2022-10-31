use anyhow::Result;
use derive_more::Constructor;

use crate::{
    flox::Flox, nix::command::BuildArgsBuilder, nix::NixArgsBuilder, prelude::Installable,
};

#[derive(Constructor)]
pub struct Package<'flox> {
    flox: &'flox Flox<'flox>,
    installable: Installable,
}

impl Package<'_> {
    /// flox build
    /// runs `nix build <installable>`
    pub async fn build(&self) -> Result<()> {
        let nix = self.flox.nix()?;

        let build_args = BuildArgsBuilder::default()
            .installables([self.installable.clone()])
            .build()?;

        let nix_args = NixArgsBuilder::default()
            .command(Box::new(build_args))
            .build()?;

        nix.run(nix_args).await?;
        Ok(())
    }
}
