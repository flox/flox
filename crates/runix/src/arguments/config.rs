use derive_builder::Builder;
use derive_more::{Deref, From};

use crate::command_line::{Flag, FlagType, ToArgs};

/// These arguments correspond to nix config settings as defined in `nix.conf` or overridden on the commandline
/// and refer to the options defined in
/// - All implementations of Setting<_> ([approximation](https://cs.github.com/?scopeName=All+repos&scope=&q=repo%3Anixos%2Fnix+%2FSetting%3C%5Cw%2B%3E%2F))
#[derive(Builder, Clone, Default)]
#[builder(setter(strip_option, into))]
pub struct NixConfig {
    accept_flake_config: Option<AcceptFlakeConfig>,
    warn_dirty: Option<WarnDirty>,
    extra_experimental_features: Option<ExperimentalFeatures>,
    extra_substituters: Option<Substituters>,
}

impl ToArgs for NixConfig {
    fn args(&self) -> Vec<String> {
        vec![
            self.accept_flake_config.args(),
            self.warn_dirty.args(),
            self.extra_experimental_features.args(),
            self.extra_substituters.args(),
            // self.extra_substituters.as_ref().map(ToArgs::args),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// flag for warn dirty
#[derive(Clone, From)]
pub struct WarnDirty;
impl Flag<Self> for WarnDirty {
    const FLAG: &'static str = "--warn-dirty";
    const FLAG_TYPE: &'static FlagType<Self> = &FlagType::Bool;
}

/// Flag for accept-flake-config
#[derive(Clone, From)]
pub struct AcceptFlakeConfig;
impl Flag<Self> for AcceptFlakeConfig {
    const FLAG: &'static str = "--accept-flake-config";
    const FLAG_TYPE: &'static FlagType<Self> = &FlagType::Bool;
}

/// Flag for extra experimental features
#[derive(Clone, From)]
pub struct ExperimentalFeatures(Vec<String>);
impl Flag<Self> for ExperimentalFeatures {
    const FLAG: &'static str = "--extra-experimental-features";
    const FLAG_TYPE: &'static FlagType<Self> = &FlagType::Args(|s| s.0.to_vec());
}

/// Flag for extra substituters
#[derive(Clone, From)]
pub struct Substituters(Vec<String>);
impl Flag<Self> for Substituters {
    const FLAG: &'static str = "--extra-substituters";
    const FLAG_TYPE: &'static FlagType<Self> = &FlagType::Args(|s| s.0.to_vec());
}
