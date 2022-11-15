use derive_more::{Deref, From};

use crate::{
    arguments::{eval::EvaluationArgs, flake::FlakeArgs, DevelopArgs, InstallablesArgs},
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

impl NixCliCommand<TemplateFlag> for FlakeInit {
    const SUBCOMMAND: &'static [&'static str] = &["flake", "init"];

    fn flake_args(&self) -> Option<FlakeArgs> {
        Some(self.flake.clone())
    }
    fn eval_args(&self) -> Option<EvaluationArgs> {
        Some(self.eval.clone())
    }

    fn own(&self) -> Option<TemplateFlag> {
        self.template.clone()
    }
}

/// `nix develop` Command
#[derive(Debug, Default, Clone)]
pub struct Develop {
    pub flake: FlakeArgs,
    pub eval: EvaluationArgs,
    pub installables: InstallablesArgs,
    pub develop_args: DevelopArgs,
}

impl NixCliCommand for Develop {
    const SUBCOMMAND: &'static [&'static str] = &["develop"];
    const FLAKE_ARGS: fn(Self) -> Option<FlakeArgs> = |s| Some(s.flake);
    const EVAL_ARGS: fn(Self) -> Option<EvaluationArgs> = |s| Some(s.eval);
    const INSTALLABLES: fn(Self) -> Option<InstallablesArgs> = |s| Some(s.installables);
}
