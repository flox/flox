use std::path::PathBuf;

use derive_more::{Deref, From};
use runix_derive::ToArgs;

use self::common::NixCommonArgs;
use self::config::NixConfigArgs;
use crate::command_line::ToArgs;
use crate::default::flag::{Flag, FlagType};
use crate::installable::Installable;

pub mod common;
pub mod config;
pub mod eval;
pub mod flake;
pub mod source;

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Debug, Default)]
pub struct NixArgs {
    /// Configure the cwd for nix actions.
    ///
    /// Relevant for instance for init and relative installables
    pub cwd: Option<PathBuf>,

    /// Common arguments to the nix command
    pub common: NixCommonArgs,

    /// Nix configuration (overrides nix.conf)
    pub config: NixConfigArgs,
}

impl ToArgs for NixArgs {
    fn to_args(&self) -> Vec<String> {
        [self.config.to_args(), self.common.to_args()]
            .into_iter()
            .flatten()
            .collect()
    }
}

/// Installable argument for commands taking a single Installable
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(From, Clone, Default, Debug)]
#[from(forward)]
pub struct InstallableArg(Option<Installable>);
impl ToArgs for InstallableArg {
    fn to_args(&self) -> Vec<String> {
        self.0.iter().map(|i| i.to_nix()).collect()
    }
}

/// Installable argument for commands taking multiple Installables
/// ([approximately](https://github.com/NixOS/nix/search?q=InstallablesCommand)
#[derive(Debug, From, Default, Clone)]
#[from(forward)]
pub struct InstallablesArgs(Vec<Installable>);
impl ToArgs for InstallablesArgs {
    fn to_args(&self) -> Vec<String> {
        self.0.iter().map(|i| i.to_nix()).collect()
    }
}

/// Nix arguments
/// should be a proper struct + de/serialization to and from [&str]
#[derive(Debug, Default, Clone, ToArgs)]
pub struct DevelopArgs {}

#[derive(Clone, From, Deref, Debug)]
#[from(forward)]
pub struct Bundler(Installable);
impl Flag for Bundler {
    const FLAG: &'static str = "--bundler";
    const FLAG_TYPE: FlagType<Self> = FlagType::arg();
}

#[derive(Debug, Default, Clone, ToArgs)]
pub struct BundleArgs {
    pub bundler: Option<Bundler>,
}

#[derive(Clone, From, Deref, Debug, Default)]
#[from(forward)]
pub struct Apply(String);
impl Flag for Apply {
    const FLAG: &'static str = "--apply";
    const FLAG_TYPE: FlagType<Self> = FlagType::arg();
}

/// Options to `nix eval`
/// https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/nix/eval.cc#LL21-40
#[derive(Debug, Default, Clone, ToArgs)]
pub struct EvalArgs {
    pub apply: Option<Apply>,
    pub installable: Option<InstallableArg>,
}
