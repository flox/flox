use std::{collections::HashMap, ops::Deref, process::Stdio};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use derive_builder::Builder;
use derive_more::Constructor;
use log::debug;
use tokio::process::Command;

use crate::{
    arguments::{
        common::NixCommonArgs, config::NixConfig, eval::EvaluationArgs, flake::FlakeArgs,
        InstallablesArgs, NixArgs,
    },
    command::NixCommand,
    NixApi,
};

/// Nix Implementation based on the Nix Command Line
#[derive(Constructor, Builder, Default, Clone)]
pub struct NixCommandLine {
    nix_bin: Option<String>,

    /// Environment
    environment: HashMap<String, String>,
    common_args: NixCommonArgs,
    flake_args: FlakeArgs,
    eval_args: EvaluationArgs,
    config: NixConfig,
}

impl NixCommandLine {
    pub async fn run_in_nix(&self, args: &Vec<&str>) -> Result<String> {
        let output = Command::new(self.nix_bin.as_deref().unwrap_or("nix"))
            .envs(&self.environment)
            .args(args)
            .output()
            .await?;

        let nix_response = std::str::from_utf8(&output.stdout)?;
        let nix_err_response = std::str::from_utf8(&output.stderr)?;

        if !nix_err_response.is_empty() {
            println!(
                "Error in nix response, {}, {}",
                nix_err_response,
                nix_err_response.len()
            );
            Err(anyhow!(
                "FXXXX: Error in nix response, {}, {}",
                nix_err_response,
                nix_err_response.len()
            ))
        } else {
            Ok(nix_response.to_string())
        }
    }
}

#[async_trait]
impl NixApi for NixCommandLine {
    /// Construct and run a nix command
    /// Merge with default argumens as applicable
    async fn run(&self, args: NixArgs) -> Result<()> {
        let mut command = Command::new(self.nix_bin.as_deref().unwrap_or("nix"));
        command
            .envs(&self.environment)
            .args(self.config.args())
            .args(self.common_args.args())
            .args(args.args())
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        debug!("Invoking nix CLI: env={:?}; {:#?}", self.environment, args);

        let mut child = command.spawn()?;

        let _ = child.wait().await?;

        // let nix_response = std::str::from_utf8(&output.stdout)?;
        // let nix_err_response = std::str::from_utf8(&output.stderr)?;

        // if !nix_err_response.is_empty() {
        //     println!(
        //         "Error in nix response, {}, {}",
        //         nix_err_response,
        //         nix_err_response.len()
        //     );
        //     Err(anyhow!(
        //         "FXXXX: Error in nix response, {}, {}",
        //         nix_err_response,
        //         nix_err_response.len()
        //     ))
        // } else {
        //     dbg!(output);
        //     Ok(())
        // }
        Ok(())
    }
}

pub trait ToArgs {
    fn args(&self) -> Vec<String>;
}

impl ToArgs for dyn NixCommand + Send + Sync {
    fn args(&self) -> Vec<String> {
        let mut acc = Vec::new();
        acc.append(&mut self.subcommand());
        acc.append(&mut self.flake_args().map_or(Vec::new(), |a| a.args()));
        acc.append(&mut self.eval_args().map_or(Vec::new(), |a| a.args()));
        acc.append(&mut self.installables().map_or(Vec::new(), |a| a.args()));
        acc
        //  ++; self.eval_args() ++ self.installables()
    }
}

/// Setting Flag Container akin to https://cs.github.com/NixOS/nix/blob/499e99d099ec513478a2d3120b2af3a16d9ae49d/src/libutil/config.cc#L199
///
/// Usage:
/// 1. Create a struct for a flag and implement [Flag] for it
/// 2. Implement [TypedFlag] for the setting or manualy implement [ToArgs]
pub trait Flag {
    const FLAG: &'static str;
}

///
pub enum FlagTypes<T> {
    Bool,
    List(fn(&T) -> Vec<String>),
}

pub trait TypedFlag: Flag
where
    Self: Sized,
{
    const FLAG_TYPE: FlagTypes<Self>;
}

impl<D: Deref<Target = Vec<String>> + Flag> TypedFlag for D {
    const FLAG_TYPE: FlagTypes<Self> = FlagTypes::List(|s| s.deref().to_owned());
}

impl<W: TypedFlag> ToArgs for W {
    fn args(&self) -> Vec<String> {
        match Self::FLAG_TYPE {
            FlagTypes::Bool => vec![Self::FLAG.to_string()],
            FlagTypes::List(f) => {
                vec![Self::FLAG.to_string(), f(self).join(" ")]
            }
        }
    }
}

impl<T: ToArgs> ToArgs for Option<T> {
    fn args(&self) -> Vec<String> {
        self.iter().map(|t| t.args()).flatten().collect()
    }
}
