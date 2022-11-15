use std::path::PathBuf;

use runix::{
    arguments::{common::NixCommonArgs, config::NixConfigArgs},
    command_line::{DefaultArgs, NixCommandLine},
    installable::Installable,
    NixBackend,
};

use crate::{
    actions::package::Package,
    environment::{self, build_flox_env},
    prelude::Stability,
    providers::git::GitProvider,
};

/// The main API struct for our flox implementation
///
/// A [Flox] instance serves as the context for nix invocations
/// and possibly other tools such as git.
/// As a CLI application one invocation of `flox` would run on the same instance
/// but may call different methods.
///
/// [Flox] will provide a preconfigured instance of the Nix API.
/// By default this nix API uses the nix CLI.
/// Preconfiguration includes environment variables and flox specific arguments.
#[derive(Debug)]
pub struct Flox {
    /// The directory pointing to the users flox configuration
    ///
    /// TODO: set a default in the lib or CLI?
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub data_dir: PathBuf,

    /// Whether to collect metrics of any kind
    /// (yet to be made use of)
    pub collect_metrics: bool,
}

pub trait FloxNixApi: NixBackend {
    fn new(nix_bin: String, defaults: DefaultArgs) -> Self;
}

impl FloxNixApi for NixCommandLine {
    fn new(nix_bin: String, defaults: DefaultArgs) -> NixCommandLine {
        NixCommandLine {
            nix_bin: Some(nix_bin),
            defaults,
        }
    }
}

impl Flox {
    pub fn package(&self, installable: Installable, stability: Stability) -> Package {
        Package::new(self, installable, stability)
    }

    pub fn nix<Nix: FloxNixApi>(&self) -> Nix {
        let nix_config_args = NixConfigArgs {
            extra_experimental_features: Some(
                ["nix-command", "flakes"].map(String::from).to_vec().into(),
            ),
            extra_substituters: Some(
                ["https://cache.floxdev.com?trusted=1"]
                    .map(String::from)
                    .to_vec()
                    .into(),
            ),
            ..Default::default()
        };

        let common_args = NixCommonArgs {
            ..Default::default()
        };

        let defaults = DefaultArgs {
            environment: build_flox_env(),
            common_args,
            ..Default::default()
        };

        Nix::new(environment::NIX_BIN.to_string(), defaults)
    }

    pub fn git_provider<Git: GitProvider>(&self) -> Git {
        Git::new()
    }
}
