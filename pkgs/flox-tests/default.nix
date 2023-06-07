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

    export TEMP_FLOX=$(mktemp -d)
    export PATH="${lib.makeBinPath paths}"
    export FLOX_PACKAGE="${flox}"
    export FLOX_CLI="${flox}/bin/flox"
    export TESTS_DIR=${tests-src}/tests
    export FLOX_DISABLE_METRICS=true
    export GIT_CONFIG_SYSTEM="$TEMP_FLOX/gitconfig-system"
    export GIT_CONFIG_GLOBAL="$TEMP_FLOX/gitconfig-global"
    export SSH_AUTH_SOCK="$TEMP_FLOX/ssh_agent.sock"

    git config --global gpg.format ssh
    git config --global user.signingkey ${tests-src}/tests/id_ed25519.pub

    exec -a "$0" bats \
        --print-output-on-failure \
        --verbose-run \
        --timing \
            ${tests-src}/tests "$@"
  ''
