use crate::command_line::ToArgs;

/// These arguments do not depend on the nix subcommand issued
/// and refer to the options defined in
/// - (libmain/common-args.cc)[https://github.com/NixOS/nix/blob/a6239eb5700ebb85b47bb5f12366404448361f8d/src/libmain/common-args.cc#L7-L81]
/// - (nix/main.cc)[https://github.com/NixOS/nix/blob/b7e8a3bf4cbb2448db860f65ea13ef2c64b6883b/src/nix/main.cc#L66-L110]
#[derive(Clone, Default, Debug)]
pub struct NixCommonArgs {}
impl ToArgs for NixCommonArgs {
    fn to_args(&self) -> Vec<String> {
        vec![]
    }
}
