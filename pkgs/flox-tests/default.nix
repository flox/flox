{
  self,
  lib,
  bash,
  bats,
  coreutils,
  entr,
  expect,
  findutils,
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
    findutils
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
in
  writeScriptBin "flox-tests" ''
        #!${bash}/bin/bash

        if command -v flox &> /dev/null
        then
          export TMP_FLOX_CLI=$(command -v flox)
        fi

        export TEMP_FLOX=$(mktemp -d)
        export PATH="${lib.makeBinPath paths}"
        export FLOX_PACKAGE="flox"
        export FLOX_DISABLE_METRICS=true
        export GIT_CONFIG_SYSTEM="$TEMP_FLOX/gitconfig-system"
        export GIT_CONFIG_GLOBAL="$TEMP_FLOX/gitconfig-global"
        export SSH_AUTH_SOCK="$TEMP_FLOX/ssh_agent.sock"

        usage() {
              cat << EOF
    Usage: $0 [--flox <FLOX BINARY>| -F <FLOX BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -F, --flox          Path to flox binary (Default: flox)
        -T, --tests         Path to folder of tests (Default: $PWD/tests)
        -W, --watch         Run tests in a continuous watch mode
        -h, --help          Prints help information
    EOF
        }

        # Transform long options to short ones
        for arg in "$@"; do
          shift
          case "$arg" in
            '--flox')   set -- "$@" '-F';;
            '--tests')  set -- "$@" '-T';;
            '--watch')  set -- "$@" '-W';;
            '--help')   set -- "$@" '-h';;
            *)          set -- "$@" "$arg" ;;
          esac
        done

        WATCH=0

        while getopts ":F:WhT::" flag;
        do
          case "$flag" in
            F)
              export FLOX_CLI=$OPTARG
              ;;
            T)
              export TESTS_DIR=$OPTARG
              ;;
            W)
              WATCH=1
              ;;
            h)
              usage
              exit 0
              ;;
          esac
        done

        # remove options from positional parameters
        if [ "$0" != "--" ];
        then
          shift $(expr $OPTIND - 2)
        fi

        # Default flag values
        if [ -z "$FLOX_CLI" ];
        then
          export FLOX_CLI=$TMP_FLOX_CLI
        fi
        if [ -z "$TESTS_DIR" ];
        then
          export TESTS_DIR=$PWD/tests
        fi

        # DEBUG
        #echo "WATCH: $WATCH"
        #echo "FLOX_CLI: $FLOX_CLI"
        #echo "TESTS_DIR: $TESTS_DIR"
        #echo "BATS ARGS: $@"

        # produce nice error when bats args are not passed after --
        if [[ ! -z "$@" ]];
        then
          if [ "$1" == "--" ];
          then
            shift
          else
            >&2 echo "ERROR: provide BATS ARGUMENTS after \`--\`!"
            exit 1
          fi
        fi

        # isolate git config
        git config --global gpg.format ssh
        if [[ -f "$TESTS_DIR/id_ed25519.pub" ]];
        then
          git config --global user.signingkey $TESTS_DIR/id_ed25519.pub
        fi

        # run basts either via entr or just a single run
        if [[ $WATCH -eq 1 ]];
        then
          find $TESTS_DIR $FLOX_CLI | entr -s '     \
            bats                                    \
              --print-output-on-failure             \
              --verbose-run                         \
              --timing                              \
                  $TESTS_DIR "$@"                 \
          ';
        else
          exec -a "$0" bats                         \
              --print-output-on-failure             \
              --verbose-run                         \
              --timing                              \
                  $TESTS_DIR "$@"
        fi
  ''
