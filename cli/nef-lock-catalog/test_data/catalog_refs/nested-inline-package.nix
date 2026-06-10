# Pattern: inline sub-package defined inside the `let` block alongside catalog
# refs.  Tests that the walker finds catalog refs in the outer let even when a
# nested `buildPythonPackage { ... }` node sits nearby in the tree.
{
  buildPythonPackage,
  catalogs,
  fetchurl,
  grpcio,
  protobuf,
  psycopg2,
  setuptools,
}:

let
  src = ../../../..;
  inherit (catalogs.myorg.toolkit) readVersion;

  inherit (catalogs.myorg.python3Packages)
    alpha-lib
    gamma-service
    theta-worker
    ;

  # inline sub-package with a pinned upstream tarball
  inline-dep = buildPythonPackage {
    pname = "inline-dep";
    version = "2.0.1";
    pyproject = true;

    src = fetchurl {
      url = "https://example.com/packages/inline-dep-2.0.1.tar.gz";
      sha256 = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    };

    build-system = [ setuptools ];

    # relaxed grpcio pin
    propagatedBuildInputs = [
      grpcio
      protobuf
      setuptools
    ];
  };

in
buildPythonPackage {
  pname = "complex-service";
  inherit src;
  version = readVersion "${src}/setup.py";
  pyproject = true;
  build-system = [ setuptools ];

  propagatedBuildInputs = [
    alpha-lib
    gamma-service
    theta-worker
    inline-dep
    psycopg2
  ];

  doCheck = false;
}
