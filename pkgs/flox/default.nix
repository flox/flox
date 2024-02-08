{
  inputs,
  self,
  lib,
  symlinkJoin,
  makeBinaryWrapper,
  flox-pkgdb,
  flox-cli,
  flox-manpages,
}: let
  # Inherit version from Cargo.toml, aligning with the CLI version.
  # We also inject some indication about the `git' revision of the repository.
  cargoToml = lib.importTOML (
    if builtins.pathExists "${inputs.flox-latest}/cli/flox/Cargo.toml"
    then "${inputs.flox-latest}/cli/flox/Cargo.toml"
    else "${inputs.flox-latest}/crates/flox/Cargo.toml"
  );
  revCountDiff = self.revCount - inputs.flox-latest.revCount;
  suffix =
    if self ? revCount && self ? shortRev
    then "${builtins.toString revCountDiff}-g${self.shortRev}"
    else "dirty";
  version = "${cargoToml.package.version}-${suffix}";
in
  symlinkJoin {
    name = "${flox-cli.pname}-${version}";

    paths = [flox-cli flox-manpages];
    nativeBuildInputs = [makeBinaryWrapper];

    postBuild = ''
      wrapProgram $out/bin/flox \
        --set PKGDB_BIN       "${flox-pkgdb}/bin/pkgdb" \
        --set LD_FLOXLIB      "${flox-pkgdb}/lib/ld-floxlib.so" \
        --set FLOX_BIN        "${flox-cli}/bin/flox" \
        --set FLOX_VERSION    "${version}"
    '';
  }
