use crate::{
    command_line::{Flag, FlagType, ToArgs},
    installable::FlakeRef,
};

/// Flake related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/84cc7ad77c6faf1cda8f8a10f7c12a939b61fe35/src/libcmd/installables.cc#L26-L126)
#[derive(Clone, Default)]
pub struct FlakeArgs {
    pub override_inputs: Vec<InputOverride>,
}

impl ToArgs for FlakeArgs {
    fn args(&self) -> Vec<String> {
        self.override_inputs
            .iter()
            .flat_map(ToArgs::args)
            .collect::<Vec<String>>()
    }
}

/// Tuple like override inputs flag
#[derive(Clone)]
pub struct InputOverride {
    pub from: FlakeRef,
    pub to: FlakeRef,
}

impl Flag<Self> for InputOverride {
    const FLAG: &'static str = "--override-input";
    const FLAG_TYPE: FlagType<Self> = FlagType::Args(Self::args);
}

impl InputOverride {
    fn args(&self) -> Vec<String> {
        dbg!(vec![self.from.clone(), self.to.clone()])
    }
}
