use derive_more::{Deref, From};

use crate::{
    arguments::{eval::EvaluationArgs, flake::FlakeArgs, DevelopArgs, InstallablesArgs},
    command_line::{
        flag::{Flag, FlagType},
        Group, NixCliCommand, TypedCommand,
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

    const INSTALLABLES: Group<Self, InstallablesArgs> = Some(|d| d.installables.clone());
    const FLAKE_ARGS: Group<Self, FlakeArgs> = Some(|d| d.flake.clone());
    const EVAL_ARGS: Group<Self, EvaluationArgs> = Some(|d| d.eval.clone());
}

impl TypedCommand for Build {
    type Output = ();
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

impl NixCliCommand<Option<TemplateFlag>> for FlakeInit {
    const SUBCOMMAND: &'static [&'static str] = &["flake", "init"];
    const INSTALLABLES: Group<Self, InstallablesArgs> = Some(|d| d.installables.clone());
    const FLAKE_ARGS: Group<Self, FlakeArgs> = Some(|d| d.flake.clone());
    const EVAL_ARGS: Group<Self, EvaluationArgs> = Some(|d| d.eval.clone());
    const OWN_ARGS: Group<Self, Option<TemplateFlag>> = Some(|d| d.template.clone());
}

/// `nix develop` Command
#[derive(Debug, Default, Clone)]
pub struct Develop {
    pub flake: FlakeArgs,
    pub eval: EvaluationArgs,
    pub installables: InstallablesArgs,
    pub develop_args: DevelopArgs,
}

impl NixCliCommand<DevelopArgs> for Develop {
    const SUBCOMMAND: &'static [&'static str] = &["develop"];
    const INSTALLABLES: Group<Self, InstallablesArgs> = Some(|d| d.installables.clone());
    const FLAKE_ARGS: Group<Self, FlakeArgs> = Some(|d| d.flake.clone());
    const EVAL_ARGS: Group<Self, EvaluationArgs> = Some(|d| d.eval.clone());
    const OWN_ARGS: Group<Self, DevelopArgs> = Some(|d| d.develop_args.clone());
}
