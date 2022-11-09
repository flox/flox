use derive_builder::Builder;

use crate::arguments::{eval::EvaluationArgs, flake::FlakeArgs, InstallablesArgs};

pub trait NixCommand {
    fn subcommand(&self) -> Vec<String>;
    fn flake_args(&self) -> Option<FlakeArgs> {
        None
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        None
    }
    fn installables(&self) -> Option<InstallablesArgs> {
        None
    }
}

#[derive(Builder, Default, Clone)]
#[builder(default)]
pub struct Build {
    flake: FlakeArgs,
    eval: EvaluationArgs,
    #[builder(setter(into))]
    installables: InstallablesArgs,
}

impl NixCommand for Build {
    fn subcommand(&self) -> Vec<String> {
        vec!["build".to_owned()]
    }

    fn flake_args(&self) -> Option<FlakeArgs> {
        Some(self.flake.clone())
    }

    fn eval_args(&self) -> Option<EvaluationArgs> {
        Some(self.eval.clone())
    }

    fn installables(&self) -> Option<InstallablesArgs> {
        Some(self.installables.clone())
    }
}
