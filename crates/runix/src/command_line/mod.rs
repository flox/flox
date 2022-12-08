use core::fmt;
use std::{
    collections::HashMap,
    ffi::OsStr,
    io,
    process::{Output, Stdio},
};

use async_trait::async_trait;

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
    pub extra_args: Vec<String>,
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
    #[error("Error running Nix: {0}")]
    Run(std::io::Error),
    #[error("Bad exit: {0:?}")]
    Exit(std::process::Output),
}

impl NixCommandLine {
    async fn run<S: AsRef<OsStr>>(
        &self,
        args: impl IntoIterator<Item = S>,
    ) -> Result<std::process::Output, NixCommandLineError> {
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
            "Invoking nix CLI:\nenv = {env:?}\nargs = {args:#?}",
            env = self.defaults.environment,
            args = args,
        );

        if log::log_enabled!(target: "posix", log::Level::Debug) {
            eprintln!(
                "+ \x1b[1m{env_string} {executable} {command_string}\x1b[0m",
                env_string = command
                    .as_std()
                    .get_envs()
                    .map(|(k, v)| {
                        format!(
                            "{}={:?}",
                            k.to_string_lossy(),
                            v.map_or("".into(), OsStr::to_string_lossy)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
                executable = shell_escape::escape(command.as_std().get_program().to_string_lossy()),
                command_string = command
                    .as_std()
                    .get_args()
                    .map(|arg| shell_escape::escape(arg.to_string_lossy()))
                    .collect::<Vec<_>>()
                    .join(" ")
            )
        }

        let output = command.output().await.map_err(NixCommandLineError::Run)?;

        if !output.status.success() {
            return Err(NixCommandLineError::Exit(output));
        }

        Ok(output)
    }

    // Small wrapping helper function to make Run implementations simpler
    async fn run_command<A, B: NixCliCommand<Own = A>>(
        &self,
        command: &B,
        nix_args: &NixArgs,
        json: bool,
    ) -> Result<std::process::Output, NixCommandLineRunError> {
        let args = vec![
            self.defaults.config_args.to_args(),
            self.defaults.common_args.to_args(),
            nix_args.to_args(),
            B::SUBCOMMAND.iter().map(ToString::to_string).collect(),
            if json {
                vec!["--json".to_string()]
            } else {
                vec![]
            },
            self.defaults.extra_args.clone(),
            command.args(),
        ];

        Ok(self.run(args.into_iter().flatten()).await?)
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
        backend.run_command(self, nix_args, false).await?;
        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum NixCommandLineRunJsonError<E> {
    #[error("Error decoding json: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Run(E),
}

#[async_trait]
impl<C> RunJson<NixCommandLine> for C
where
    C: NixCliCommand + JsonCommand + Send + Sync,
{
    type JsonError = NixCommandLineRunJsonError<Self::Error>;

    async fn run_json(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<Value, Self::JsonError> {
        let output = backend
            .run_command(self, nix_args, true)
            .await
            .map_err(NixCommandLineRunJsonError::Run)?;

        let out_str = String::from_utf8_lossy(&output.stdout);

        debug!("JSON command output: {:?}", out_str);

        Ok(serde_json::from_str(&out_str)?)
    }
}

#[async_trait]
impl<C> RunTyped<NixCommandLine> for C
where
    C: RunJson<NixCommandLine> + TypedCommand + Send + Sync,
    <C as TypedCommand>::Output: for<'de> Deserialize<'de>,
{
    type Output = C::Output;

    type TypedError = <Self as RunJson<NixCommandLine>>::JsonError;

    async fn run_typed(
        &self,
        backend: &NixCommandLine,
        nix_args: &NixArgs,
    ) -> Result<Self::Output, Self::TypedError> {
        match self.run_json(backend, nix_args).await {
            Ok(v) => Ok(serde_json::from_value(v).unwrap()),
            Err(e) => Err(e),
        }
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
