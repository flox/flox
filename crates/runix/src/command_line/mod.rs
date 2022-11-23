use core::fmt;
use std::{
    borrow::Cow, collections::HashMap, error::Error, ffi::OsStr, io, marker::PhantomData,
    ops::Deref, process::Stdio,
};

use async_trait::async_trait;
use derive_more::Constructor;
use log::debug;
use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;
use tokio::process::Command;

use crate::{
    arguments::{
        common::NixCommonArgs, config::NixConfigArgs, eval::EvaluationArgs, flake::FlakeArgs,
        source::SourceArgs, InstallableArg, InstallablesArgs, NixArgs,
    },
    command::Develop,
    NixBackend, Run, RunJson, RunTyped,
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

pub trait ToArgs {
    fn to_args(&self) -> Vec<String>;
}

impl<T: ToArgs> ToArgs for Option<T> {
    fn to_args(&self) -> Vec<String> {
        self.iter().flat_map(|t| t.to_args()).collect()
    }
}

impl<T: ToArgs> ToArgs for Vec<T> {
    fn to_args(&self) -> Vec<String> {
        self.iter().flat_map(|t| t.to_args()).collect()
    }
}

pub type Group<T, U> = Option<fn(&T) -> U>;

pub trait NixCliCommand: fmt::Debug + Sized {
    type Own: ToArgs;

    const SUBCOMMAND: &'static [&'static str];

    const INSTALLABLES: Group<Self, InstallablesArgs> = None;
    const INSTALLABLE: Group<Self, InstallableArg> = None;
    const FLAKE_ARGS: Group<Self, FlakeArgs> = None;
    const EVAL_ARGS: Group<Self, EvaluationArgs> = None;
    const SOURCE_ARGS: Group<Self, SourceArgs> = None;
    const OWN_ARGS: Group<Self, Self::Own> = None;

    fn args(&self) -> Vec<String> {
        let mut acc = Vec::new();
        acc.append(&mut Self::FLAKE_ARGS.map_or(Vec::new(), |f| f(self).to_args()));
        acc.append(&mut Self::EVAL_ARGS.map_or(Vec::new(), |f| f(self).to_args()));
        acc.append(&mut Self::SOURCE_ARGS.map_or(Vec::new(), |f| f(self).to_args()));
        acc.append(&mut Self::INSTALLABLES.map_or(Vec::new(), |f| f(self).to_args()));
        acc.append(&mut Self::INSTALLABLE.map_or(Vec::new(), |f| f(self).to_args()));
        acc.append(&mut Self::OWN_ARGS.map_or(Vec::new(), |f| f(self).to_args()));
        acc
    }
}

/// Marker Trait for commands that can return JSON
///
/// Used to automatically implement [RunJson] for the implementer
/// by adding `--json` to the nix backend invocation
///
/// Commands that may output json data but doing so other than with `--json`
/// should implment [RunJson] directly instead of this marker.
pub trait JsonCommand {}

/// Marker Trait for commands that can be deserialized into
/// [TypedCommand::Output]
///
/// Used to automatically implement [RunTyped] for implementers
/// that implement [RunJson] by trying to deserialize
/// the json output to its associated type
///
/// Commands that may be deserialized from other data than JSON
/// should implment [RunTyped] directly instead of this marker
/// eg for [Develop]:
///
/// ```
/// use async_trait::async_trait;
/// use runix::arguments::NixArgs
/// use runix::command::Develop
/// use runix::{RunTyped, NixBackend}
///
/// struct NixCommandLine;
/// impl NixBackend for NixCommandLine {}
///
/// #[async_trait]
/// impl RunTyped<NixCommandLine> for Develop {
///     type Output = ();
///     async fn run_typed(
///         &self,
///         backend: &NixCommandLine,
///         nix_args: &NixArgs,
///     ) -> Result<Self::Output, Self::Error> {
///         todo!()
///     }
/// }
/// ```

pub trait TypedCommand {
    type Output;
}

impl ToArgs for () {
    fn to_args(&self) -> Vec<String> {
        Default::default()
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

    async fn run(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<(), NixCommandLineRunError> {
        let args = [
            backend.defaults.config_args.to_args(),
            backend.defaults.common_args.to_args(),
            nix_args.to_args(),
            Self::SUBCOMMAND.iter().map(ToString::to_string).collect(),
            self.args(),
        ]
        .into_iter()
        .flatten();

        backend.run(args).await?;
        Ok(())
    }
}

#[async_trait]
impl<C> RunJson<NixCommandLine> for C
where
    C: Run<NixCommandLine> + JsonCommand + Send + Sync,
{
    async fn json(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<Value, Self::Error> {
        if let Ok(v) = self.run(backend, nix_args).await {
            return Ok(serde_json::from_str(unimplemented!()).unwrap());
        }
        todo!()
    }
}

#[async_trait]
impl<C> RunTyped<NixCommandLine> for C
where
    C: RunJson<NixCommandLine> + TypedCommand + Send + Sync,
    <C as TypedCommand>::Output: for<'de> Deserialize<'de>,
{
    type Output = C::Output;
    async fn run_typed(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<Self::Output, Self::Error> {
        if let Ok(v) = self.json(backend, nix_args).await {
            return Ok(serde_json::from_value(v).unwrap());
        }

        todo!()
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
