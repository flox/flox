use derive_more::{Deref, From};

use crate::{
    arguments::{eval::EvaluationArgs, flake::FlakeArgs, InstallablesArgs},
    command_line::{
        flag::{Flag, FlagType},
        NixCliCommand, ToArgs,
    },
    installable::Installable,
};

pub trait NixJsonCommand: NixCliCommand {}

#[derive(Debug, Default, Clone)]
pub struct Build {
    pub flake: FlakeArgs,
    pub eval: EvaluationArgs,
    pub installables: InstallablesArgs,
}

impl NixCliCommand for Build {
    const SUBCOMMAND: &'static [&'static str] = &["build"];

    fn flake_args(&self) -> Option<FlakeArgs> {
        Some(self.flake.clone())
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        Some(self.eval.clone())
    }

    fn installables(&self) -> Option<InstallablesArgs> {
        Some(self.installables.clone())
    }

    fn own(&self) -> Option<Vec<String>> {
        None
    }
}

/// `nix flake init` Command
#[derive(Debug, Default, Clone)]
pub struct FlakeInit {
    pub flake: FlakeArgs,
    pub eval: EvaluationArgs,
    pub installables: InstallablesArgs,

    pub template: Option<TemplateFlag>,
}

#[derive(Deref, Debug, Clone, From)]
#[from(forward)]
pub struct TemplateFlag(Installable);
impl Flag for TemplateFlag {
    const FLAG: &'static str = "--template";
    const FLAG_TYPE: FlagType<Self> = FlagType::arg();
}

impl NixCliCommand for FlakeInit {
    const SUBCOMMAND: &'static [&'static str] = &["flake", "init"];

    fn flake_args(&self) -> Option<FlakeArgs> {
        Some(self.flake.clone())
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        Some(self.eval.clone())
    }

    fn own(&self) -> Option<Vec<String>> {
        self.template.as_ref().map(ToArgs::args)
    }
}
