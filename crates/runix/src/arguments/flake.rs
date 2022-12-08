use derive_more::{Constructor, Deref, From};

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
    pub no_write_lock_file: NoWriteLockFile,
}

impl ToArgs for FlakeArgs {
    fn to_args(&self) -> Vec<String> {
        vec![
            self.no_write_lock_file.to_args(),
            self.override_inputs.to_args(),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

/// Tuple like override inputs flag
#[derive(Clone, Debug, From, Constructor)]
pub struct OverrideInputs {
    pub from: FlakeRef,
    pub to: FlakeRef,
}
impl Flag for OverrideInputs {
    const FLAG: &'static str = "--override-input";
    const FLAG_TYPE: FlagType<Self> = FlagType::Args(Self::args);
}
impl OverrideInputs {
    fn args(&self) -> Vec<String> {
        vec![self.from.clone(), self.to.clone()]
    }
}

/// Flag for no-write-lock-file
#[derive(Clone, From, Debug, Deref, Default)]
pub struct NoWriteLockFile(bool);
impl Flag for NoWriteLockFile {
    const FLAG: &'static str = "--no-write-lock-file";
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}
