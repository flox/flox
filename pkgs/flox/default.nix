{
  symlinkJoin,
  makeWrapper,
  flox-pkgdb,
  flox-env-builder,
  flox-cli,
}:
symlinkJoin {
  name = flox-cli.name;

  paths = [flox-cli];
  nativeBuildInputs = [makeWrapper];

  postBuild = ''
    wrapProgram $out/bin/flox \
      --set PKGDB_BIN       ${flox-pkgdb}/bin/pkgdb \
      --set ENV_BUILDER_BIN ${flox-env-builder}/bin/env-builder \
      --set FLOX_BIN        ${flox-cli}/bin/flox
  '';
}
