{
  inputs,
  self,
  lib,
  symlinkJoin,
  makeWrapper,
  flox-pkgdb,
  flox-env-builder,
  flox-cli,
}: let
  # Inherit version from Cargo.toml, aligning with the CLI version.
  # We also inject some indication about the `git' revision of the repository.
  cargoToml = lib.importTOML "${inputs.flox-latest}/cli/flox/Cargo.toml";
  revCountDiff = self.revCount - inputs.flox-latest.revCount;
  revCount =
    if revCountDiff == 0
    then ""
    else "${toString revCountDiff}-";
  suffix =
    if self ? revCount && self ? shortRev
    then "${revCount}g${self.shortRev}"
    else "dirty";
  version = "${cargoToml.package.version}-${suffix}";
in
  symlinkJoin {
    name = flox-cli.name;

    paths = [flox-cli];
    nativeBuildInputs = [makeWrapper];

    postBuild = ''
      wrapProgram $out/bin/flox \
        --set PKGDB_BIN       "${flox-pkgdb}/bin/pkgdb" \
        --set ENV_BUILDER_BIN "${flox-env-builder}/bin/env-builder" \
        --set FLOX_BIN        "${flox-cli}/bin/flox" \
        --set FLOX_VERSION    "${version}"
    '';
  }
