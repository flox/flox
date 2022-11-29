use std::{default, path::PathBuf};

use derive_more::{Deref, From};

use crate::command_line::{
    flag::{Flag, FlagType},
    ToArgs,
};

/// These arguments correspond to nix config settings as defined in `nix.conf` or overridden on the commandline
/// and refer to the options defined in
/// - All implementations of Setting<_> ([approximation](https://cs.github.com/?scopeName=All+repos&scope=&q=repo%3Anixos%2Fnix+%2FSetting%3C%5Cw%2B%3E%2F))
#[derive(Clone, Default, Debug)]
pub struct NixConfigArgs {
    pub accept_flake_config: AcceptFlakeConfig,
    pub warn_dirty: WarnDirty,
    pub flake_registry: Option<FlakeRegistry>,
    pub extra_experimental_features: ExperimentalFeatures,
    pub extra_substituters: Substituters,
    pub show_trace: ShowTrace,
}

impl ToArgs for NixConfigArgs {
    fn to_args(&self) -> Vec<String> {
        vec![
            self.accept_flake_config.to_args(),
            self.warn_dirty.to_args(),
            self.extra_experimental_features.to_args(),
            self.extra_substituters.to_args(),
            self.flake_registry.to_args(),
            self.show_trace.to_args(),
            // self.extra_substituters.as_ref().map(ToArgs::args),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// flag for warn dirty
#[derive(Clone, From, Debug, Deref, Default)]
pub struct WarnDirty(bool);
impl Flag for WarnDirty {
    const FLAG: &'static str = "--warn-dirty";
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}

/// Flag for accept-flake-config
#[derive(Clone, From, Debug, Deref, Default)]
pub struct AcceptFlakeConfig(bool);
impl Flag for AcceptFlakeConfig {
    const FLAG: &'static str = "--accept-flake-config";
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}

/// Flag for show-trace
#[derive(Clone, From, Debug, Deref, Default)]
pub struct ShowTrace(bool);
impl Flag for ShowTrace {
    const FLAG: &'static str = "--show-trace";
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}

/// Flag for extra experimental features
#[derive(Clone, From, Deref, Debug, Default)]
pub struct ExperimentalFeatures(Vec<String>);
impl Flag for ExperimentalFeatures {
    const FLAG: &'static str = "--extra-experimental-features";
    const FLAG_TYPE: FlagType<Self> = FlagType::list();
}

/// Flag for extra substituters
#[derive(Clone, From, Deref, Debug, Default)]
pub struct Substituters(Vec<String>);
impl Flag for Substituters {
    const FLAG: &'static str = "--extra-substituters";
    const FLAG_TYPE: FlagType<Self> = FlagType::list();
}

/// Flag for extra substituters
#[derive(Clone, From, Deref, Debug, Default)]
pub struct FlakeRegistry(PathBuf);
impl Flag for FlakeRegistry {
    const FLAG: &'static str = "--flake-registry";
    const FLAG_TYPE: FlagType<Self> = FlagType::Args(|s| vec![s.0.to_string_lossy().to_string()]);
}
