# Pattern: direct `catalogs.org.pkg` Select assignments for native (non-Python)
# packages, alongside inherit-from for toolkit + python3Packages.
{
  buildPythonPackage,
  catalogs,
  gnupg,
  setuptools,
}:

let
  src = ../../../..;
  inherit (catalogs.myorg.toolkit) readMakeVersion;

  inherit (catalogs.myorg.python3Packages) epsilon-core;

  proxy-wrap = catalogs.myorg.proxy-wrap;
  queue-bin = catalogs.myorg.queue-bin;

in
buildPythonPackage {
  pname = "secure-client";
  inherit src;
  version = readMakeVersion "${src}/Makefile";
  pyproject = true;
  build-system = [ setuptools ];

  nativeBuildInputs = [ gnupg ];

  postPatch = ''
    substituteInPlace src/secure_client/paths.py \
      --replace-fail /usr/bin/proxy-wrap ${proxy-wrap}/bin/proxy-wrap \
      --replace-fail /usr/bin/queue-bin  ${queue-bin}/bin/queue-bin
  '';

  propagatedBuildInputs = [ epsilon-core ];

  doCheck = false;
}
