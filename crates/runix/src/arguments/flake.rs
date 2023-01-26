use derive_more::{Constructor, Deref, From};
use runix_derive::ToArgs;

use crate::command_line::flag::{Flag, FlagType};
use crate::command_line::ToArgs;
use crate::installable::FlakeRef;

/// Flake related arguments
/// Corresponding to the arguments defined in
/// [libcmd/installables.cc](https://github.com/NixOS/nix/blob/84cc7ad77c6faf1cda8f8a10f7c12a939b61fe35/src/libcmd/installables.cc#L26-L126)
#[derive(Clone, Default, Debug, ToArgs)]
pub struct FlakeArgs {
    pub override_inputs: Vec<OverrideInput>,
    pub no_write_lock_file: NoWriteLockFile,
}

/// Tuple like override inputs flag
#[derive(Clone, Debug, From, Constructor)]
pub struct OverrideInput {
    pub from: FlakeRef,
    pub to: FlakeRef,
}
impl Flag for OverrideInput {
    const FLAG: &'static str = "--override-input";
    const FLAG_TYPE: FlagType<Self> = FlagType::Args(Self::args);
}
impl OverrideInput {
    fn args(&self) -> Vec<String> {
        vec![self.from.clone(), self.to.clone()]
    }
}

/// Flag for no-write-lock-file
#[derive(Clone, From, Debug, Deref, Default)]
pub struct NoWriteLockFile(bool);
impl Flag for NoWriteLockFile {
    const FLAG: &'static str = "--no-write-lock-file";
    /// Not a config/switch.
    /// There is no `--write-lock-file` equivalent
    const FLAG_TYPE: FlagType<Self> = FlagType::bool();
}
