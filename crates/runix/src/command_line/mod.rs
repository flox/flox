use core::fmt;
use std::{
    collections::HashMap, error::Error, ffi::OsStr, io, marker::PhantomData, ops::Deref,
    process::Stdio,
};

use async_trait::async_trait;
use derive_more::Constructor;
use log::debug;
use thiserror::Error;
use tokio::process::Command;

use crate::{
    arguments::{
        common::NixCommonArgs, config::NixConfigArgs, eval::EvaluationArgs, flake::FlakeArgs,
        InstallablesArgs, NixArgs,
    },
    NixBackend, Run,
};

pub mod flag;

#[derive(Clone, Debug, Default)]
pub struct DefaultArgs {
    pub environment: HashMap<String, String>,
    pub common_args: NixCommonArgs,
    pub config_args: NixConfigArgs,
    pub flake_args: FlakeArgs,
    pub eval_args: EvaluationArgs,
}

/// Nix Implementation based on the Nix Command Line
#[derive(Clone, Debug, Default)]
pub struct NixCommandLine {
    pub nix_bin: Option<String>,
    pub defaults: DefaultArgs,
}

#[derive(Error, Debug)]
pub enum NixCommandLineError {
    #[error("Nix printed {0} bytes to stderr")]
    Printed(u32),
    #[error("Failed to spawn nix")]
    Spawn(std::io::Error),
    #[error("Failed to wait for Nix")]
    Wait(std::io::Error),
}

impl NixCommandLine {
    async fn run<S: AsRef<OsStr>>(
        &self,
        args: impl IntoIterator<Item = S>,
    ) -> Result<(), NixCommandLineError> {
        let mut command = Command::new(self.nix_bin.as_deref().unwrap_or("nix"));
        command
            .envs(&self.defaults.environment)
            .args(args)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        let args = command
            .as_std()
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect::<Vec<_>>();

        debug!(
            "Invoking nix CLI: env={:?}; {:#?}",
            self.defaults.environment, args
        );

        let mut child = command.spawn().map_err(NixCommandLineError::Spawn)?;

        let _ = child.wait().await.map_err(NixCommandLineError::Wait)?;

        Ok(())
    }
}

pub trait IntoArgs {
    fn into_args(&self) -> Vec<String>;
}

impl<T: IntoArgs> IntoArgs for Option<T> {
    fn into_args(&self) -> Vec<String> {
        self.iter().flat_map(|t| t.into_args()).collect()
    }
}

pub trait NixCliCommand: fmt::Debug + Sized {
    const SUBCOMMAND: &'static [&'static str];

    const FLAKE_ARGS: fn(Self) -> Option<FlakeArgs> = |_| None;
    const EVAL_ARGS: fn(Self) -> Option<EvaluationArgs> = |_| None;
    const INSTALLABLES: fn(Self) -> Option<InstallablesArgs> = |_| None;

    fn flake_args(&self) -> Option<FlakeArgs> {
        None
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        None
    }
    fn installables(&self) -> Option<InstallablesArgs> {
        None
    }
    fn own(&self) -> Option<Vec<String>> {
        None
    }

    fn args(&self) -> Vec<String> {
        let mut acc = Vec::new();
        acc.append(&mut self.flake_args().map_or(Vec::new(), |a| a.into_args()));
        acc.append(&mut self.eval_args().map_or(Vec::new(), |a| a.into_args()));
        acc.append(&mut self.installables().map_or(Vec::new(), |a| a.into_args()));
        acc.append(&mut self.own().unwrap_or_default());
        acc
    }
}

impl NixBackend for NixCommandLine {}

#[derive(Error, Debug)]
pub enum NixCommandLineRunError {
    #[error("An error orrured in CommandLine backend: {0}")]
    Backend(#[from] NixCommandLineError),
}

#[async_trait]
impl<C> Run<NixCommandLine> for C
where
    C: NixCliCommand + Send + Sync,
{
    type Error = NixCommandLineRunError;

    // type Backend = NixCommandLine;

    async fn run(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<(), NixCommandLineRunError> {
        let args = [
            backend.defaults.config_args.into_args(),
            backend.defaults.common_args.into_args(),
            nix_args.into_args(),
            Self::SUBCOMMAND.iter().map(ToString::to_string).collect(),
            self.args(),
        ]
        .into_iter()
        .flatten();

        backend.run(args).await?;
        Ok(())
    }
}

impl NixBackend for u32 {}

#[async_trait]
impl<C> Run<u32> for C
where
    C: NixCliCommand + Send + Sync,
{
    type Error = io::Error;

    // type Backend = NixCommandLine;

    async fn run(&self, backend: &u32, nix_args: &NixArgs) -> Result<(), io::Error> {
        panic!("42")
        // backend.run_in_nix(args)
    }
}
