{
  self,
  lib,
  bash,
  bats,
  coreutils,
  entr,
  expect,
  flox,
  gawk,
  git,
  gnugrep,
  gnupg,
  gnused,
  gnutar,
  jq,
  nix-editor,
  openssh,
  parallel,
  unixtools,
  which,
  writeScriptBin,
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths = [
    bash
    batsWith
    coreutils
    entr
    expect
    flox
    gawk
    git
    gnupg
    gnugrep
    gnused
    gnutar
    jq
    nix-editor
    openssh
    parallel
    unixtools.util-linux
    which
  ];

  tests-src = builtins.path {
    name = "flox-tests-src";
    path = self;
    filter = path: type:
      ! builtins.elem path [
        (self.outPath + "/flake.nix")
        (self.outPath + "/flake.lock")
        (self.outPath + "/pkgs")
        (self.outPath + "/checks")
        (self.outPath + "/shells")
        (self.outPath + "/crates")
      ];
  };
in
  writeScriptBin "flox-tests" ''
    #!${bash}/bin/bash

    export PATH="${lib.makeBinPath paths}"
    export FLOX_PACKAGE="${flox}"
    export FLOX_CLI="${flox}/bin/flox"
    export TESTS_DIR=${tests-src}/tests
    export FLOX_DISABLE_METRICS=true

    exec -a "$0" bats \
        --print-output-on-failure \
        --verbose-run \
        --timing \
            ${tests-src}/tests "$@"
  ''
