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
