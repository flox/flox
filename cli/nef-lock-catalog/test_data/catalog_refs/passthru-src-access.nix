# Pattern: a catalog-derived native package is bound to a variable, and then
# its `passthru.src` is accessed in a string interpolation (${native-pkg.src}/...).
# That string interpolation must NOT produce a spurious catalog ref — only the
# original `catalogs.myorg.queue-bin` assignment should be recorded.
{
  buildPythonPackage,
  catalogs,
  go,
  grpcio,
  grpcio-tools,
  protobuf,
  pyyaml,
  setuptools,
}:

let
  src = ../../../..;
  inherit (catalogs.myorg.toolkit) readVersion;

  inherit (catalogs.myorg.python3Packages)
    gamma-service
    zeta-api
    ;

  queue-bin = catalogs.myorg.queue-bin;

in
buildPythonPackage {
  pname = "queue-py";
  inherit src;
  version = readVersion "${src}/setup.py";
  pyproject = true;

  build-system = [
    setuptools
    grpcio-tools
  ];

  postPatch = ''
    mkdir -p ./adapters/queue
    cp ${queue-bin.src}/proto/service.proto ./adapters/queue/service.proto
    substituteInPlace setup.py --replace-fail '/usr/bin/go' '${go}/bin/go'
  '';

  propagatedBuildInputs = [
    gamma-service
    grpcio
    protobuf
    pyyaml
    zeta-api
  ];

  passthru.src = src;

  doCheck = false;

  meta.description = "Python gRPC client for queue-bin";
}
