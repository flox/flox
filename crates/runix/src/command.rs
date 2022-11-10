use crate::{
    arguments::{eval::EvaluationArgs, flake::FlakeArgs, InstallablesArgs},
    command_line::ToArgs,
};

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

#[derive(Default, Clone)]
pub struct Build {
    pub flake: FlakeArgs,
    pub eval: EvaluationArgs,
    pub installables: InstallablesArgs,
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
