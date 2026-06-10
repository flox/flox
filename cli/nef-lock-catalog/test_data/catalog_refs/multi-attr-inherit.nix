# Pattern: inherit-from with multiple attribute names spanning several lines,
# plus a toolkit helper — the form that produced truncated output before the fix.
{
  buildPythonPackage,
  catalogs,
  setuptools,
  click,
  requests,
}:

let
  src = ../../../..;
  inherit (catalogs.myorg.toolkit) readVersion;

  inherit (catalogs.myorg.python3Packages)
    alpha-lib
    delta-util
    epsilon-core
    eta-parser
    theta-worker
    ;

in
buildPythonPackage {
  pname = "large-service";
  inherit src;
  version = readVersion "${src}/setup.py";
  pyproject = true;
  build-system = [ setuptools ];

  propagatedBuildInputs = [
    alpha-lib
    delta-util
    epsilon-core
    eta-parser
    theta-worker
    click
    requests
  ];

  doCheck = false;
}
