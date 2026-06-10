# Pattern: `inherit (someAttrSet.sub) attr;` where the base is NOT catalogs.
# These should produce zero catalog refs.  Exercises two non-catalog inherits:
#   - inherit from a `builtins` result (pyprojectAttrs)
#   - inherit from a local let-binding (buildAttrs)
{
  buildPythonPackage,
  setuptools,
}:

let
  src = ../../../..;
  pyprojectAttrs = builtins.fromTOML (builtins.readFile "${src}/pyproject.toml");
  buildAttrs = {
    system = "x86_64-linux";
    backend = "setuptools";
  };

in
buildPythonPackage {
  pname = "pure-nixpkgs-pkg";
  inherit src;
  inherit (pyprojectAttrs.project) version;
  inherit (buildAttrs) system;
  pyproject = true;
  build-system = [ setuptools ];

  doCheck = false;
}
