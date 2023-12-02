{
  self,
  lib,
  bats,
  nix,
  gnugrep,
  jq,
  sqlite,
  git,
  yj,
  bash,
  coreutils,
  flox-pkgdb,
  writeShellScriptBin,
  testsDir ? "/tests",
  PKGDB ? "${flox-pkgdb}/bin/pkgdb",
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths = [
    bash
    yj
    coreutils
    gnugrep
    batsWith
    git
    nix
    jq
    sqlite
  ];
in
  writeShellScriptBin "flox-pkgdb-tests" ''
    set -x -euo pipefail

    export PATH="${lib.makeBinPath paths}"

    # copy checkout to temporary directory
    WORKDIR=$(mktemp -d -t flox-pkgdb-tests-XXXXXX)
    cp -R ${self}/pkgdb/* $WORKDIR
    cd $WORKDIR

    usage() {
          cat << EOF
    Usage: $0 [--pkgdb <PKGDB BINARY>| -P <PKGDB BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -P, --pkgdb         Path to pkgdb binary (Default: pkgdb)
        -T, --tests         Path to folder of tests (Default: $PWD/pkgdb/${testsDir})
        -W, --watch         Run tests in a continuous watch mode
        -h, --help          Prints help information
    EOF
    }

    ${
      if PKGDB == null
      then ""
      else "export PKGDB='${PKGDB}';"
    }

    WATCH=;
    declare -a _TESTS;
    _TESTS=();
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        -[pP]|--pkgdb)        export PKGDB="''${2?}"; shift; ;;
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

    export PKGDB;
    export IS_SQLITE3="${flox-pkgdb.test}/bin/is_sqlite3"
    export SEARCH_PARAMS="${flox-pkgdb.test}/bin/search-params";

    # Default flag values
    : "''${TESTS_DIR:=$PWD${testsDir}}";
    export TESTS_DIR PKGDB;

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
      echo "  PKGDB:        $PKGDB";
      echo "  TESTS_DIR:    $TESTS_DIR";
      echo "  tests:        ''${_TESTS[*]}";
      echo "  bats options: ''${_BATS_ARGS[*]}";
      echo "  bats command: bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    } >&2;

    # run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$PKGDB"  \
        |entr -s "bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    else
      exec -a "$0" bats "''${_BATS_ARGS[@]}" "''${_TESTS[@]}";
    fi
  ''
