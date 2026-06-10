# Pattern: no `catalogs` argument — pure nixpkgs package with no catalog refs.
{
  buildPythonPackage,
  fetchPypi,
  requests,
  setuptools,
}:

buildPythonPackage rec {
  pname = "standalone-lib";
  version = "1.4.2";
  pyproject = true;

  src = fetchPypi {
    inherit pname version;
    sha256 = "sha256-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa==";
  };

  build-system = [ setuptools ];

  propagatedBuildInputs = [ requests ];

  doCheck = false;
}
