use derive_builder::Builder;
use derive_more::From;

use crate::{command::NixCommand, command_line::ToArgs, installable::Installable};

use self::{common::NixCommonArgs, config::NixConfig};

pub mod common;
pub mod config;
pub mod eval;
pub mod flake;

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Builder)]
#[builder(pattern = "owned")]
pub struct NixArgs {
    /// Common arguments to the nix command
    #[builder(default)]
    common: NixCommonArgs,

    /// Nix configuration (overrides nix.conf)
    #[builder(default)]
    config: NixConfig,

    /// Arguments to the nix subcommand
    /// These may contain flake/evaluation args if applicable
    // #[builder(setter(skip))]
    command: Box<dyn NixCommand + Send + Sync>,
}

impl ToArgs for NixArgs {
    fn args(&self) -> Vec<String> {
        let mut acc = vec![];
        acc.append(&mut self.config.args());
        acc.append(&mut self.common.args());
        acc.append(&mut (*self.command.as_ref()).args());
        acc
    }
}

/// Installable argument for commands taking a single Installable
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Clone)]
pub struct InstallableArg(Installable);

/// Installable argument for commands taking multiple Installables
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Default, Clone)]
#[from(forward)]
pub struct InstallablesArgs(Vec<Installable>);

impl ToArgs for InstallablesArgs {
    fn args(&self) -> Vec<String> {
        self.0.iter().map(|i| i.to_nix()).collect()
    }
}
