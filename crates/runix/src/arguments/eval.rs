use crate::command_line::flag::{Flag, FlagType};
use crate::command_line::ToArgs;
use derive_more::{Deref, From};

/// Evaluation related arguments
/// Corresponding to the arguments defined in
/// [libcmd/common-eval-args.cc](https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libcmd/common-eval-args.cc#L14-L74)
#[derive(Clone, Default, Debug)]
pub struct EvaluationArgs {
    pub impure: Impure,
}

impl ToArgs for EvaluationArgs {
    fn to_args(&self) -> Vec<String> {
        vec![self.impure.to_args()].into_iter().flatten().collect()
    }
}

#[derive(Clone, From, Debug, Deref, Default)]
pub struct Impure(bool);
impl Flag for Impure {
    const FLAG: &'static str = "--impure";
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}
