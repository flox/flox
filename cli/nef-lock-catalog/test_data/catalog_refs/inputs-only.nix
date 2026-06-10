# Pattern: `inputs.*` references only — no `catalogs` argument.
# Exercises both inherit-from and direct Select forms for flake inputs.
{
  inputs,
  stdenv,
}:

let
  # inherit a sub-attrset from one flake input
  inherit (inputs.nixpkgs) lib;

  # direct select for a package output of another flake input
  helper-bin = inputs.devtools-flake.packages.default;

in
stdenv.mkDerivation {
  pname = "flake-input-pkg";
  version = lib.fileContents "${inputs.self}/VERSION";
  src = inputs.self;
  buildInputs = [ helper-bin ];
  doCheck = false;
}
