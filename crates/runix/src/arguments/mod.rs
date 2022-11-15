use derive_more::From;

use crate::{command_line::IntoArgs, installable::Installable};

use self::{common::NixCommonArgs, config::NixConfigArgs};

pub mod common;
pub mod config;
pub mod eval;
pub mod flake;

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Debug, Default)]
pub struct NixArgs {
    /// Common arguments to the nix command
    common: NixCommonArgs,

    /// Nix configuration (overrides nix.conf)
    config: NixConfigArgs,
}

impl IntoArgs for NixArgs {
    fn into_args(&self) -> Vec<String> {
        let mut acc = vec![];
        acc.append(&mut self.config.into_args());
        acc.append(&mut self.common.into_args());
        acc
    }
}

/// Installable argument for commands taking a single Installable
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Clone)]
pub struct InstallableArg(Installable);

/// Installable argument for commands taking multiple Installables
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(Debug, From, Default, Clone)]
#[from(forward)]
pub struct InstallablesArgs(Vec<Installable>);

impl IntoArgs for InstallablesArgs {
    fn into_args(&self) -> Vec<String> {
        self.0.iter().map(|i| i.to_nix()).collect()
    }
}

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Debug, Default, Clone)]
pub struct DevelopArgs {}

impl IntoArgs for DevelopArgs {
    fn into_args(&self) -> Vec<String> {
        let mut acc = vec![];

        acc
    }
}
