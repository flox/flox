use std::path::PathBuf;

use derive_more::{Deref, From};
use runix_derive::ToArgs;

use crate::command_line::flag::{Flag, FlagType};
use crate::command_line::ToArgs;

/// These arguments correspond to nix config settings as defined in `nix.conf` or overridden on the commandline
/// and refer to the options defined in
/// - All implementations of Setting<_> ([approximation](https://cs.github.com/?scopeName=All+repos&scope=&q=repo%3Anixos%2Fnix+%2FSetting%3C%5Cw%2B%3E%2F))
#[derive(Clone, Default, Debug, ToArgs)]
pub struct NixConfigArgs {
    pub accept_flake_config: AcceptFlakeConfig,
    pub connect_timeout: ConnectTimeout,
    pub extra_access_tokens: AccessTokens,
    pub extra_experimental_features: ExperimentalFeatures,
    pub extra_substituters: Substituters,
    pub extra_trusted_public_keys: TrustedPublicKeys,
    pub flake_registry: Option<FlakeRegistry>,
    pub netrc_file: Option<NetRCFile>,
    pub show_trace: ShowTrace,
    pub warn_dirty: WarnDirty,
}

impl NixConfigArgs {
    fn config_items(&self) -> Vec<(String, String)> {
        [
            self.accept_flake_config.to_config(),
            self.connect_timeout.to_config(),
            self.extra_access_tokens.to_config(),
            self.extra_experimental_features.to_config(),
            self.extra_substituters.to_config(),
            self.extra_trusted_public_keys.to_config(),
            self.flake_registry.as_ref().and_then(ToConfig::to_config),
            self.netrc_file.as_ref().and_then(ToConfig::to_config),
            self.show_trace.to_config(),
            self.warn_dirty.to_config(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub fn to_config_string(&self) -> String {
        self.config_items()
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

trait ToConfig {
    fn to_config(&self) -> Option<(String, String)>;
}

impl<T> ToConfig for T
where
    T: Flag,
{
    fn to_config(&self) -> Option<(String, String)> {
        let name = &T::FLAG[2..];
        match T::FLAG_TYPE {
            FlagType::Switch(default, f) => {
                let value = f(self);
                (value != default).then_some((name.to_owned(), value.to_string()))
            },
            FlagType::Indicator(f) => f(self).then_some((name.to_owned(), true.to_string())),
            _ => match self.to_args()[..] {
                [] | [_] => None,
                ref args => Some((name.to_owned(), args[1..].join(" "))),
            },
        }
    }
}

/// flag for warn dirty
#[derive(Clone, From, Debug, Deref, Default)]
pub struct WarnDirty(bool);
impl Flag for WarnDirty {
    const FLAG: &'static str = "--warn-dirty";
    const FLAG_TYPE: FlagType<Self> = FlagType::switch(true);
}

/// Flag for accept-flake-config
#[derive(Clone, From, Debug, Deref, Default)]
pub struct AcceptFlakeConfig(bool);
impl Flag for AcceptFlakeConfig {
    const FLAG: &'static str = "--accept-flake-config";
    const FLAG_TYPE: FlagType<Self> = FlagType::switch(false);
}

/// Flag for accept-flake-config
#[derive(Clone, From, Debug, Deref, Default)]
pub struct ConnectTimeout(u32);
impl Flag for ConnectTimeout {
    const FLAG: &'static str = "--connect-timeout";
    const FLAG_TYPE: FlagType<Self> = FlagType::number_arg();
}

/// Flag for show-trace
#[derive(Clone, From, Debug, Deref, Default)]
pub struct ShowTrace(bool);
impl Flag for ShowTrace {
    const FLAG: &'static str = "--show-trace";
    /// technically a switch (`--no-show-trace` seems to be allowed)
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
    const FLAG_TYPE: FlagType<Self> = FlagType::os_str_arg();
}

/// Flag for extra substituters
#[derive(Clone, From, Deref, Debug, Default)]
pub struct NetRCFile(PathBuf);
impl Flag for NetRCFile {
    const FLAG: &'static str = "--netrc-file";
    const FLAG_TYPE: FlagType<Self> = FlagType::os_str_arg();
}

/// Flag for extra access tokens
#[derive(Clone, From, Deref, Debug, Default)]
pub struct AccessTokens(Vec<(String, String)>);
impl Flag for AccessTokens {
    const FLAG: &'static str = "--extra-access-tokens";
    const FLAG_TYPE: FlagType<Self> = FlagType::map();
}

/// Flag for extra trusted public keys
#[derive(Clone, From, Deref, Debug, Default)]
pub struct TrustedPublicKeys(Vec<String>);
impl Flag for TrustedPublicKeys {
    const FLAG: &'static str = "--extra-trusted-public-keys";
    const FLAG_TYPE: FlagType<Self> = FlagType::list();
}
