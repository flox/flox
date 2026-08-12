# `myorg.pkg` is two components as written here but already package-deep once
# re-rooted under `catalogs.myorg`, so it is a package being passed, not a
# namespace being forwarded.
{ myorg }:
{
  viaDeep = (import ./deep.nix { pkg = myorg.pkg; }).version;
}
