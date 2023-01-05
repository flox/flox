use derive_more::{Deref, From};
use runix_derive::ToArgs;

use crate::command_line::ToArgs;
use crate::default::flag::{Flag, FlagType};

/// Source installable related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libcmd/installables.cc#L146-L186)
#[derive(Clone, Default, Debug, ToArgs)]
pub struct SourceArgs {
    pub expr: Option<Expr>,
}

#[derive(Clone, From, Deref, Debug, Default)]
pub struct Expr(String);
impl Flag for Expr {
    const FLAG: &'static str = "--expr";
    const FLAG_TYPE: FlagType<Self> = FlagType::arg();
}
