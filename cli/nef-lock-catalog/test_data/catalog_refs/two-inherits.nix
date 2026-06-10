# Pattern: two separate inherit-from statements — one for a toolkit helper
# and one for a single python3Packages entry.
{
  buildPythonPackage,
  catalogs,
  setuptools,
  requests,
}:

let
  src = ../../../../client;

  inherit (catalogs.myorg.toolkit) readVersion;
  inherit (catalogs.myorg.python3Packages) beta-client;

in
buildPythonPackage {
  pname = "gamma-service";
  inherit src;
  version = readVersion "${src}/setup.py";
  pyproject = true;
  build-system = [ setuptools ];

  propagatedBuildInputs = [
    beta-client
    requests
  ];

  doCheck = false;
}
