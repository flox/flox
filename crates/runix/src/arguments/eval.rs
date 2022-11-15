use crate::command_line::IntoArgs;

/// Evaluation related arguments
/// Corresponding to the arguments defined in
/// [libcmd/common-eval-args.cc](https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libcmd/common-eval-args.cc#L14-L74)
#[derive(Clone, Default, Debug)]
pub struct EvaluationArgs {}

impl IntoArgs for EvaluationArgs {
    fn into_args(&self) -> Vec<String> {
        vec![]
    }
}
