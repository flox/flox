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
}: let
  # Inherit version from Cargo.toml, aligning with the CLI version.
  # We also inject some indication about the `git' revision of the repository.
  cargoToml = (lib.importTOML "${inputs.self}/cli/flox/Cargo.toml").package.version or "dirty";
  cargoTomlLatest = (lib.importTOML "${inputs.flox-latest}/cli/flox/Cargo.toml").package.version or "dirty";
  revCountDiff = self.revCount - inputs.flox-latest.revCount;
  version =
    if !(self ? revCount || self ? shortRev)
    then # path://$PWD
      "${cargoToml}-dirty"
    else if !(self ? revCount)
    then # github:flox/flox
      "${cargoToml}-g${self.shortRev}"
    else if revCountDiff == 0
    then # for release, only possible with overrides/follows
      "${cargoToml}"
    else # git+ssh://git@github.com/flox/flox
      "${cargoTomlLatest}-${builtins.toString revCountDiff}-g${self.shortRev}";
in
  symlinkJoin {
    name = "${flox-cli.pname}-${version}";
    inherit version;

    paths = [flox-cli flox-manpages];
    nativeBuildInputs = [makeBinaryWrapper];

    postBuild = ''
      wrapProgram $out/bin/flox \
        ${lib.optionalString (SENTRY_DSN != null) "--set SENTRY_DSN \"${SENTRY_DSN}\" "} \
        ${lib.optionalString (SENTRY_ENV != null) "--set SENTRY_ENV \"${SENTRY_ENV}\" "} \
        --set PKGDB_BIN       "${flox-pkgdb}/bin/pkgdb" \
        --set LD_FLOXLIB      "${flox-pkgdb}/lib/ld-floxlib.so" \
        --set FLOX_BIN        "${flox-cli}/bin/flox" \
        --set FLOX_VERSION    "${version}"
    '';
  }
