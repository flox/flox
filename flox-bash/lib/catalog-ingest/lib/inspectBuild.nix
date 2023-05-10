#
# NOTE: this file is mastered in:
#       ${inputs.flox-floxpkgs}/lib/inspectBuild.nix
#
# ... but we cannot copy from that input at build time
# because that creates a cyclic dependency which causes
# flox to be rebuilt when floxpkgs changes, which causes
# a publish of the built flox package back to floxpkgs,
# and then the cycle continues. Until we find a better
# way to prevent this (probably by putting the catalog
# data on a different branch) we'll just have to keep these
# files in sync by hand - hopefully they won't change often.
#
{
  self,
  lib,
  inputs,
}: {
  substituteOnly ? false, # requires --impure
  ...
}: outPath: let
  dir =
    (
      if substituteOnly
      then builtins.storePath
      else lib.id
    )
    outPath;
  root = builtins.readDir dir;
  local =
    if root.local or "none" == "directory"
    then builtins.readDir "${dir}/local"
    else {};
  man = local.man or "none" == "directory";
  bin = local.bin or "none" == "directory";
in {
  hasMan = man;
  hasBin = bin;
}
