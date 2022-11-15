use async_trait::async_trait;
use log::debug;
use std::{collections::HashMap, process::Stdio};
use thiserror::Error;
use tokio::process::Command;

pub trait IntoArgs {
    fn into_args(self) -> Vec<String>;
}

#[derive(Clone, Debug, Default)]
pub struct NixCommonArgs {
    pub nix_config_args: NixConfigArgs,
}

impl IntoArgs for NixCommonArgs {
    fn into_args(self) -> Vec<String> {
        self.nix_config_args.into_args()
    }
}

#[derive(Clone, Debug, Default)]
pub struct NixConfigArgs {
    pub accept_flake_config: bool,
    pub warn_dirty: bool,
    pub extra_experimental_features: Vec<String>,
    pub extra_substituters: Vec<String>,
}

impl IntoArgs for NixConfigArgs {
    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        if self.accept_flake_config {
            args.push("--accept-flake-config".to_string());
        }
        if self.warn_dirty {
            args.push("--warn-dirty".to_string());
        }
        if self.extra_experimental_features.len() > 0 {
            args.push("--extra-experimental-features".to_string());
            for feat in self.extra_experimental_features {
                args.push(feat);
            }
        }
        if self.extra_substituters.len() > 0 {
            args.push("--extra-substituters".to_string());
            for sub in self.extra_substituters {
                args.push(sub);
            }
        }
        args
    }
}

#[derive(Clone, Debug, Default)]
pub struct EvaluationArgs {}

impl IntoArgs for EvaluationArgs {
    fn into_args(self) -> Vec<String> {
        vec![]
    }
}

#[derive(Clone, Debug)]
pub struct InputOverride {
    pub from: String,
    pub to: String,
}

impl IntoArgs for InputOverride {
    fn into_args(self) -> Vec<String> {
        vec!["--override-input".to_string(), self.from, self.to]
    }
}

#[derive(Clone, Debug, Default)]
pub struct FlakeArgs {
    pub override_inputs: Vec<InputOverride>,
}

impl IntoArgs for FlakeArgs {
    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        for sub in self.override_inputs {
            args.append(&mut sub.into_args());
        }
        args
    }
}

#[derive(Debug, Clone)]
pub struct Installable {
    pub flakeref: String,
    pub attr_path: String,
}

impl From<String> for Installable {
    fn from(input: String) -> Self {
        let mut split = input.splitn(2, '#');

        match (split.next(), split.next()) {
            (Some(flakeref), Some(attr_path)) => Installable {
                flakeref: flakeref.to_owned(),
                attr_path: attr_path.to_owned(),
            },
            (Some(attr_path), None) => Installable {
                flakeref: ".".to_owned(),
                attr_path: attr_path.to_owned(),
            },
            _ => unreachable!(),
        }
    }
}

impl std::fmt::Display for Installable {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}#{}", self.flakeref, self.attr_path)
    }
}

fn installable_args<I: IntoIterator<Item = Installable>>(i: I) -> Vec<String> {
    i.into_iter().map(|x| x.to_string()).collect()
}

#[derive(Clone, Debug, Default)]
pub struct BuildArgs {
    pub common_args: NixCommonArgs,
    pub flake_args: FlakeArgs,
    pub eval_args: EvaluationArgs,
    pub installables: Vec<Installable>,
}

impl IntoArgs for BuildArgs {
    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        args.append(&mut self.common_args.into_args());
        args.append(&mut self.flake_args.into_args());
        args.append(&mut self.eval_args.into_args());
        args.append(&mut installable_args(self.installables));
        args
    }
}

#[derive(Clone, Debug, Default)]
pub struct EvalArgs {
    pub common_args: NixCommonArgs,
    pub flake_args: FlakeArgs,
    pub eval_args: EvaluationArgs,
    pub installable: Option<Installable>,
}

impl IntoArgs for EvalArgs {
    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        args.append(&mut self.common_args.into_args());
        args.append(&mut self.flake_args.into_args());
        args.append(&mut self.eval_args.into_args());
        args.append(&mut installable_args(self.installable));
        args
    }
}

#[derive(Clone, Debug, Default)]
pub struct FlakeInitArgs {
    pub common_args: NixCommonArgs,
    pub flake_args: FlakeArgs,
    pub eval_args: EvaluationArgs,
    pub template: Option<Installable>,
}

impl IntoArgs for FlakeInitArgs {
    fn into_args(self) -> Vec<String> {
        let mut args = Vec::new();
        args.append(&mut self.common_args.into_args());
        args.append(&mut self.flake_args.into_args());
        args.append(&mut self.eval_args.into_args());
        if let Some(installable) = self.template {
            args.push("--template".to_string());
            args.push(installable.to_string());
        }
        args
    }
}

#[async_trait]
pub trait NixApi {
    type BuildError: Send + Sync + core::fmt::Debug + core::fmt::Display;
    type EvalError: Send + Sync + core::fmt::Debug + core::fmt::Display;
    type FlakeInitError: Send + Sync + core::fmt::Debug + core::fmt::Display;

    async fn build(&self, build_args: BuildArgs) -> Result<(), Self::BuildError>;
    async fn eval(&self, build_args: EvalArgs) -> Result<(), Self::EvalError>;
    async fn flake_init(&self, flake_init_args: FlakeInitArgs) -> Result<(), Self::FlakeInitError>;
}

#[derive(Clone, Debug, Default)]
pub struct DefaultArgs {
    pub environment: HashMap<String, String>,
    pub common_args: NixCommonArgs,
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
pub enum NixCommandLineRunError {
    #[error("Nix printed {0} bytes to stderr")]
    Printed(u32),
    #[error("Failed to spawn nix")]
    Spawn(std::io::Error),
    #[error("Failed to wait for Nix")]
    Wait(std::io::Error),
}

impl NixCommandLine {
    async fn run<A: IntoArgs>(
        &self,
        subcommand: &[&str],
        args: A,
    ) -> Result<(), NixCommandLineRunError> {
        let mut command = Command::new(self.nix_bin.as_deref().unwrap_or("nix"));
        command
            .envs(&self.defaults.environment)
            .args(subcommand)
            .args(self.defaults.common_args.clone().into_args())
            .args(args.into_args())
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

        let mut child = command.spawn().map_err(NixCommandLineRunError::Spawn)?;

        let _ = child.wait().await.map_err(NixCommandLineRunError::Wait)?;

        Ok(())
    }
}

#[derive(Error, Debug)]
pub enum NixCommandBuildError {
    #[error("Error running Nix build command")]
    NixBuild(#[from] NixCommandLineRunError),
}
#[derive(Error, Debug)]
pub enum NixCommandEvalError {
    #[error("Error running Nix eval command")]
    NixEval(#[from] NixCommandLineRunError),
}
#[derive(Error, Debug)]
pub enum NixCommandFlakeInitError {
    #[error("Error running Nix flake init command")]
    NixFlakeInit(#[from] NixCommandLineRunError),
}
#[async_trait]
impl NixApi for NixCommandLine {
    type BuildError = NixCommandLineRunError;
    type EvalError = NixCommandBuildError;
    type FlakeInitError = NixCommandFlakeInitError;

    async fn build(&self, build_args: BuildArgs) -> Result<(), Self::BuildError> {
        self.run(&["build"], build_args).await.map_err(|e| e.into())
    }
    async fn eval(&self, eval_args: EvalArgs) -> Result<(), Self::EvalError> {
        self.run(&["eval"], eval_args).await.map_err(|e| e.into())
    }
    async fn flake_init(&self, flake_init_args: FlakeInitArgs) -> Result<(), Self::FlakeInitError> {
        self.run(&["flake", "init"], flake_init_args)
            .await
            .map_err(|e| e.into())
    }
}
