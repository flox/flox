# Pattern: single `inherit (catalogs.myorg.toolkit) fn;` to pull in one
# build-helper function from the catalog toolkit package.
{
  buildPythonPackage,
  catalogs,
  setuptools,
}:

let
  src = ../../../..;
  inherit (catalogs.myorg.toolkit) readVersion;

in
buildPythonPackage {
  pname = "alpha-lib";
  inherit src;
  version = readVersion "${src}/alpha/__init__.py";
  pyproject = true;
  build-system = [ setuptools ];

  doCheck = false;
}
