use derive_more::{Constructor, From};

use crate::{
    command_line::{
        flag::{Flag, FlagType},
        ToArgs,
    },
    installable::FlakeRef,
};

/// Flake related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/84cc7ad77c6faf1cda8f8a10f7c12a939b61fe35/src/libcmd/installables.cc#L26-L126)
#[derive(Clone, Default, Debug)]
pub struct FlakeArgs {
    pub override_inputs: Vec<OverrideInputs>,
}

impl ToArgs for FlakeArgs {
    fn to_args(&self) -> Vec<String> {
        let flags = self
            .override_inputs
            .iter()
            .flat_map(ToArgs::to_args)
            .collect::<Vec<String>>();

        dbg!(flags)
    }
}

/// Tuple like override inputs flag
#[derive(Clone, Debug, From, Constructor)]
pub struct OverrideInputs {
    from: FlakeRef,
    to: FlakeRef,
}
impl Flag for OverrideInputs {
    const FLAG: &'static str = "--override-input";
    const FLAG_TYPE: FlagType<Self> = FlagType::Args(Self::args);
}
impl OverrideInputs {
    fn args(&self) -> Vec<String> {
        dbg!(vec![self.from.clone(), self.to.clone()])
    }
}
