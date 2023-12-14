{
  lib,
  writeShellScriptBin,
  symlinkJoin,
  python3,
  nix,
  flox-pkgdb,
  flox-cli,
  ci ? true,
}: let
  pyenv = python3.withPackages (ps:
    with ps; [
      pytest
      pytest-emoji
      pytest-xdist
      pexpect
      ipdb
    ]);

  runner = writeShellScriptBin "flox-end2end" ''
    set -euo pipefail

    export NIX_BIN='${nix}/bin/nix';
    export PKGDB_BIN='${flox-pkgdb}/bin/pkgdb';
    export FLOX_BIN='${flox-cli}/bin/flox';

    export PYTHONDONTWRITEBYTECODE=1

    exec -a "$0" "${pyenv}/bin/pytest" "${../../end2end}" \
      -p "no:cacheprovider" \
      --durations=0 \
      --emoji \
      -vv \
      "$@"
  '';
in
  symlinkJoin {
    name = "flox-end2end-env";
    paths =
      [
        pyenv
      ]
      ++ lib.optional ci runner;

    meta.mainProgram = "flox-end2end";
  }
