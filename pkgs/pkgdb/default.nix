{
  inputs,
  callPackage,
  flox-nix,
  ...
}:
callPackage (inputs.pkgdb + "/pkg-fun.nix") {
  nix = flox-nix;
}
