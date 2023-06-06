{
  self,
  bats,
  coreutils,
  entr,
  expect,
  flox,
  gawk,
  git,
  gnugrep,
  gnused,
  gnutar,
  jq,
  nix-editor,
  parallel,
  unixtools,
  which,
  mkShell,
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);
in
  mkShell {
    name = "flox-tests";
    buildInputs = [
      batsWith
      coreutils
      entr
      expect
      flox
      gawk
      git
      gnugrep
      gnused
      gnutar
      jq
      nix-editor
      parallel
      unixtools.util-linux
      which
    ];
    shellHook = ''
      export FLOX_PACKAGE="${flox}"
      export FLOX_CLI="${flox}/bin/flox"
      export TESTS_DIR=./tests/
      export FLOX_DISABLE_METRICS=true
    '';
  }
