use derive_more::{Deref, From};

use crate::{
    command_line::ToArgs,
    default::flag::{Flag, FlagType},
};

/// Source installable related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libcmd/installables.cc#L146-L186)
#[derive(Clone, Default, Debug)]
pub struct SourceArgs {
    pub expr: Option<Expr>,
}

impl ToArgs for SourceArgs {
    fn to_args(&self) -> Vec<String> {
        self.expr.to_args()
    }
}
#[derive(Clone, From, Deref, Debug, Default)]
pub struct Expr(String);
impl Flag for Expr {
    const FLAG: &'static str = "--expr";
    const FLAG_TYPE: FlagType<Self> = FlagType::arg();
}
