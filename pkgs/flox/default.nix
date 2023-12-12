{
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
  cargoToml = lib.importTOML ./../../cli/flox/Cargo.toml;
  versionPrefix =
    if self ? revCount
    then "r"
    else "";
  # Add `r<REV-COUNT>' if available, otherwise fallback to the short
  # revision hash or "dirty" to be added as the _tag_ property of
  # the version.
  rev = self.revCount or self.shortRev or "dirty";
  # Assemble the version string.
  floxVersion =
    cargoToml.package.version + "-" + versionPrefix + (toString rev);
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
        --set FLOX_VERSION    "${floxVersion}"
    '';
  }
