use derive_builder::Builder;
use derive_more::Constructor;

use crate::{
    command_line::{Flag, ToArgs},
    installable::FlakeRef,
};

/// Flake related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/84cc7ad77c6faf1cda8f8a10f7c12a939b61fe35/src/libcmd/installables.cc#L26-L126)
#[derive(Builder, Clone, Default)]
#[builder(setter(strip_option, into))]
pub struct FlakeArgs {
    override_inputs: Option<Vec<OverrideInputs>>,
}

impl ToArgs for FlakeArgs {
    fn args(&self) -> Vec<String> {
        let flags = self.override_inputs.as_ref().map(|overrides| {
            overrides
                .into_iter()
                .flat_map(ToArgs::args)
                .collect::<Vec<String>>()
        });

        dbg!(flags.unwrap_or_default())
    }
}

/// Tuple like override inputs flag
#[derive(Clone, Constructor)]
pub struct OverrideInputs {
    from: FlakeRef,
    to: FlakeRef,
}

impl Flag for OverrideInputs {
    const FLAG: &'static str = "--override-input";
}
impl ToArgs for OverrideInputs {
    fn args(&self) -> Vec<String> {
        dbg!(vec![
            Self::FLAG.to_string(),
            self.from.clone(),
            self.to.clone()
        ])
    }
}
