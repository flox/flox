{
  self,
  lib,
  bats,
  tree,
  coreutils,
  nix,
  git,
  flox-pkgdb,
  flox-env-builder,
  writeShellScriptBin,
  testsDir ? "/tests",
  ENV_BUILDER ? "${flox-env-builder}/bin/flox-env-builder",
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths = [
    batsWith
    coreutils
    nix
    git
    tree
  ];
in
  writeShellScriptBin "flox-env-builder-tests" ''
    set -x -euo pipefail

    export PATH="${lib.makeBinPath paths}"

    # copy checkout to temporary directory
    WORKDIR=$(mktemp -d -t flox-env-builder-tests-XXXXXX)
    cp -R ${./../..}/env-builder/* $WORKDIR
    cd $WORKDIR

    tree $WORKDIR/tests

    usage() {
          cat << EOF
    Usage: $0 [--env-builder <ENV_BUILDER BINARY>| -E <ENV_BUILDER BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -E, --env-builder   Path to env-builder binary (Default: env-builder)
        -T, --tests         Path to folder of tests (Default: $PWD/env-builder/${testsDir})
        -W, --watch         Run tests in a continuous watch mode
        -h, --help          Prints help information
    EOF
    }

    ${
      if ENV_BUILDER == null
      then ""
      else "export ENV_BUILDER='${ENV_BUILDER}';"
    }

    WATCH=;
    declare -a _TESTS;
    _TESTS=();
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        -[eE]|--env-builder)  export ENV_BUILDER="''${2?}"; shift; ;;
        -[tT]|--tests)        export TESTS_DIR="''${2?}"; shift; ;;
        -[wW]|--watch)        WATCH=:; ;;
        -h|--help|-u|--usage) usage; exit 0; ;;
        --)                   shift; break; ;;
        *)
          if [[ -e "$1" ]]; then
            _TESTS+=( "$1" );
          else
            echo "''${0##*/} ERROR: Unrecognized arg(s) '$*'" >&2;
            usage;
            exit 1;
          fi
        ;;
      esac
      shift;
    done

    export ENV_BUILDER;
    export PKGDB=${flox-pkgdb}/bin/pkgdb;

    # Default flag values
    : "''${TESTS_DIR:=$PWD${testsDir}}";
    export TESTS_DIR ENV_BUILDER;

    if [[ "''${#_TESTS[@]}" -lt 1 ]]; then
      _TESTS=( "$TESTS_DIR" );
    fi

    # Collect args/options and log them
    declare -a _BATS_ARGS;
    _BATS_ARGS=(
      '--print-output-on-failure'
      '--verbose-run'
      '--timing'
      "''${@:-}"
    );
    {
      echo "''${0##*/}: Running test suite with:";
      echo "  ENV_BUILDER:        $ENV_BUILDER";
      echo "  TESTS_DIR:    $TESTS_DIR";
      echo "  tests:        ''${_TESTS[*]}";
      echo "  bats options: ''${_BATS_ARGS[*]}";
      echo "  bats command: bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    } >&2;

    # run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$ENV_BUILDER"  \
        |entr -s "bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    else
      exec -a "$0" bats "''${_BATS_ARGS[@]}" "''${_TESTS[@]}";
    fi
  ''
