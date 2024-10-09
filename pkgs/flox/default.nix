{
  inputs,
  self,
  lib,
  symlinkJoin,
  makeBinaryWrapper,
  flox-pkgdb,
  flox-watchdog,
  flox-cli,
  flox-manpages,
  process-compose,
  pkgsFor,
  SENTRY_DSN ? null,
  SENTRY_ENV ? null,
  FLOX_VERSION ? null,
}:
let
  fileVersion = lib.fileContents "${inputs.self}/VERSION";
  # flox version is in the git descrive format (not semver)
  version =
    if (FLOX_VERSION != null) then
      FLOX_VERSION
    else if !(self ? revCount || self ? shortRev) then
      "${fileVersion}-dirty"
    else if !(self ? revCount) then
      "${fileVersion}-g${self.shortRev}"
    else
      fileVersion;
in
symlinkJoin {
  name = "flox-${version}";
  inherit version;

  paths = [
    flox-cli
    flox-watchdog
    flox-manpages
  ];
  nativeBuildInputs = [ makeBinaryWrapper ];

  postBuild = ''
    wrapProgram $out/bin/flox \
      ${lib.optionalString (SENTRY_DSN != null) "--set FLOX_SENTRY_DSN \"${SENTRY_DSN}\" "} \
      ${lib.optionalString (SENTRY_ENV != null) "--set FLOX_SENTRY_ENV \"${SENTRY_ENV}\" "} \
      --set PKGDB_BIN       "${flox-pkgdb}/bin/pkgdb" \
      --set FLOX_BIN        "${flox-cli}/bin/flox" \
      --set WATCHDOG_BIN    "${flox-watchdog}/bin/flox-watchdog" \
      --set PROCESS_COMPOSE_BIN "${process-compose}/bin/process-compose" \
      --set FLOX_VERSION    "${version}"
  '';
  passthru = {
    inherit pkgsFor;
  };
}
