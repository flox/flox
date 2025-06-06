{
  inputs,
  self,
  lib,
  nix,
  symlinkJoin,
  makeBinaryWrapper,
  flox-buildenv,
  flox-nix-plugins,
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
    else if (self ? shortRev) then
      "${fileVersion}-g${self.shortRev}"
    else if (self ? dirtyShortRev) then
      "${fileVersion}-g${self.dirtyShortRev}"
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
      --set NIX_BIN             "${nix}/bin/nix" \
      --set BUILDENV_NIX        "${flox-buildenv}/lib/buildenv.nix" \
      --set NIX_PLUGINS         "${flox-nix-plugins}/lib/nix-plugins" \
      --set WATCHDOG_BIN        "${flox-watchdog}/libexec/flox-watchdog" \
      --set PROCESS_COMPOSE_BIN "${process-compose}/bin/process-compose" \
      --set FLOX_VERSION        "${version}"

    # make sure that version can be parsed
    $out/bin/flox --version
  '';

  passthru = {
    inherit pkgsFor;
  };
}
