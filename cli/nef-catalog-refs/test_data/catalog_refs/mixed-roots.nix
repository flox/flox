# Pattern: both `catalogs.*` and `inputs.*` references in a single file.
# Searching with only one root must return only that root's refs;
# searching with both must return the union.
{
  buildPythonPackage,
  catalogs,
  inputs,
  setuptools,
  requests,
}:

let
  src = ../../../..;

  # catalog refs
  inherit (catalogs.myorg.toolkit) readVersion;
  inherit (catalogs.myorg.python3Packages) alpha-lib;

  # flake input refs
  inherit (inputs.nixpkgs) lib;
  extra-tool = inputs.devtools-flake.packages.default;

in
buildPythonPackage {
  pname = "mixed-pkg";
  inherit src;
  version = readVersion "${src}/setup.py";
  pyproject = true;
  build-system = [ setuptools ];

  propagatedBuildInputs = [
    alpha-lib
    requests
  ];

  nativeBuildInputs = [ extra-tool ];

  meta.description = lib.fakeStr "mixed catalog and input deps";

  doCheck = false;
}
