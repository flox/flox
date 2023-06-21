{
  self,
  lib,
  bash,
  zsh,
  dash,
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
  writeShellScriptBin,
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths = [
    bash
    zsh
    dash
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
  writeShellScriptBin "flox-tests" ''

        if command -v flox &> /dev/null
        then
          TMP_FLOX_CLI="$(command -v flox)"
          export TMP_FLOX_CLI
        fi

        TEMP_FLOX="$(mktemp -d)"
        export TEMP_FLOX
        export PATH="${lib.makeBinPath paths}"
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

        WATCH=
        while [[ "$#" -gt 0 ]]; do
          case "$1" in
            -[fF]|--flox)         export FLOX_CLI="''${2?}"; shift; ;;
            -[tT]|--tests)        export TESTS_DIR="''${2?}"; shift; ;;
            -[wW]|--watch)        WATCH=: ;;
            -h|--help|-u|--usage) usage exit 0 ;;
            --)                   shift; break; ;;
            *)  echo "ERROR: Unrecognized arg '$*'" >&2; usage; exit 1; ;;
          esac
          shift;
        done

        # Default flag values
        : "''${FLOX_CLI:=$TMP_FLOX_CLI}";
        : "''${TESTS_DIR:=$PWD/tests}";
        export TESTS_DIR FLOX_CLI;

        # isolate git config
        ssh-keygen -t ed25519 -q -N "" -f "$TEMP_FLOX/id_ed25519";
        # FIXME: Why on earth is the test suite modifying my GPG perms?!
        #        This took me so long to debug...
        #        I'm leaving it for now but this is high on the TODO list.
        git config --global gpg.format ssh;
        git config --global user.signingkey "$TEMP_FLOX/id_ed25519.pub";

        # run basts either via entr or just a single run
        if [[ -n "''${WATCH:-}" ]]; then
          find "$TESTS_DIR" "$FLOX_CLI"|entr -s '   \
            bats                                    \
              --print-output-on-failure             \
              --verbose-run                         \
              --timing                              \
              "''${@:-$TESTS_DIR}"                  \
          ';
        else
          exec -a "$0" bats                         \
              --print-output-on-failure             \
              --verbose-run                         \
              --timing                              \
              "''${@:-$TESTS_DIR}"                  \
          ;
        fi
  ''
