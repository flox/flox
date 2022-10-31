use std::path::PathBuf;

use crate::{
    environment::build_flox_env,
    models::catalog::Stability,
    nix::{
        command_line::NixCommandLine, EvaluationArgs, FlakeArgs, NixAPI, NixCommonArgs, NixConfig,
    },
    prelude::Installable,
};
use anyhow::Result;
use config::builder;
use derive_builder::Builder;

/// The main API struct for our flox implementation
///
/// A [Flox] instance serves as the context for nix invocations
/// and possibly other tools such as git.
/// As a CLI application one invocation of `flox` would run on the same instance
/// but may call different methods.
///
/// [Flox] will provide a preconfigured instance of the Nix API.
/// By default this nix API uses the nix CLI.
/// Preconfiguration includes environemnt variables and flox specific arguments.
#[derive(Builder)]
pub struct Flox<'flox> {
    /// The directory pointing to the users flox configuration
    ///
    /// TODO: set a default in the lib or CLI?
    config_dir: PathBuf,

    /// Whether to collect metrics of any kind
    /// (yet to be made use of)
    #[builder(default)]
    collect_metrics: bool,

    /// The stability context for this instance
    #[builder(default = "Stability::Stable")]
    stability: Stability,

    /// Additional `nix` arguments
    ///
    /// TODO: Implementation detail, should go along with the nix Configurator
    #[builder(default)]
    extra_nix_args: Vec<String>,

    #[builder(default = "&FloxNix")]
    custom_nix: &'flox dyn ConfigureNix,
}

impl Flox<'_> {
    pub fn nix(&self) -> Result<Box<dyn NixAPI>> {
        self.custom_nix.configure(self)
    }
}

pub trait ConfigureNix {
    fn configure<'nix, 'a: 'nix>(&'a self, flox: &Flox) -> Result<Box<dyn NixAPI>>;
}

struct FloxNix;

impl ConfigureNix for FloxNix {
    fn configure<'nix, 'a: 'nix>(&'a self, flox: &Flox) -> Result<Box<dyn NixAPI>> {
        Ok(Box::new(NixCommandLine::new(
            build_flox_env()?,
            NixCommonArgs::default(),
            FlakeArgs::default(),
            EvaluationArgs::default(),
            NixConfig::default(),
        )))
    }
}
