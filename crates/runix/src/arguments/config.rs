use std::collections::HashMap;
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
    pub warn_dirty: WarnDirty,
    pub flake_registry: Option<FlakeRegistry>,
    pub extra_experimental_features: ExperimentalFeatures,
    pub extra_substituters: Substituters,
    pub extra_trusted_public_keys: TrustedPublicKeys,
    pub extra_access_tokens: AccessTokens,
    pub show_trace: ShowTrace,
    pub netrc_file: Option<NetRCFile>,
    pub connect_timeout: ConnectTimeout,
}

impl NixConfigArgs {
    fn flags(&self) -> Vec<Vec<String>> {
        vec![
            self.accept_flake_config.to_args(),
            self.warn_dirty.to_args(),
            self.extra_experimental_features.to_args(),
            self.extra_substituters.to_args(),
            self.extra_trusted_public_keys.to_args(),
            self.flake_registry.to_args(),
            self.show_trace.to_args(),
            self.netrc_file.to_args(),
            self.extra_access_tokens.to_args(),
        ]
    }

    fn config_items(&self) -> Vec<(String, String)> {
        self.flags()
            .into_iter()
            .filter_map(|f| match &f[..] {
                [] => None,
                [b] => Some((b[2..].to_string(), true.to_string())),
                [l, ls @ ..] => Some((l[2..].to_string(), ls.join(" "))),
            })
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
pub struct AccessTokens(HashMap<String, String>);
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
