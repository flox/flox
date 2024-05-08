{
  inputs,
  self,
  lib,
  symlinkJoin,
  makeBinaryWrapper,
  flox-pkgdb,
  flox-cli,
  flox-manpages,
  SENTRY_DSN ? null,
  SENTRY_ENV ? null,
  VERSION ? null,
}: let
  version =
    if (VERSION == null) then VERSION
    else lib.fileContents "${inputs.self}/VERSION";
in
  symlinkJoin {
    name = "${flox-cli.pname}-${version}";
    inherit version;

    paths = [flox-cli flox-manpages];
    nativeBuildInputs = [makeBinaryWrapper];

    postBuild = ''
      wrapProgram $out/bin/flox \
        ${lib.optionalString (SENTRY_DSN != null) "--set FLOX_SENTRY_DSN \"${SENTRY_DSN}\" "} \
        ${lib.optionalString (SENTRY_ENV != null) "--set FLOX_SENTRY_ENV \"${SENTRY_ENV}\" "} \
        --set PKGDB_BIN       "${flox-pkgdb}/bin/pkgdb" \
        --set LD_FLOXLIB      "${flox-pkgdb}/lib/ld-floxlib.so" \
        --set FLOX_BIN        "${flox-cli}/bin/flox" \
        --set FLOX_VERSION    "${version}"
    '';
  }
