{
  self,
  lib,
  entr,
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
  PROJECT_NAME ? "flox-pkgdb-tests",
  PROJECT_TESTS_DIR ? ./../../pkgdb/tests,
  PKGDB_BIN ? "${flox-pkgdb}/bin/pkgdb",
  PKGDB_IS_SQLITE3_BIN ? "${flox-pkgdb.test}/bin/is_sqlite3",
  PKGDB_SEARCH_PARAMS_BIN ? "${flox-pkgdb.test}/bin/search-params",
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
  # TODO: we should run tests against different shells
  writeShellScriptBin PROJECT_NAME ''
    set -euo pipefail

    # Find root of the subproject if not specified
    PROJECT_TESTS_DIR=${PROJECT_TESTS_DIR}
    PROJECT_PATH=""
    if [[ $PROJECT_TESTS_DIR != "/nix/store/"* ]]; then

      # Find top level of the project
      if ${git}/bin/git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
        export PROJECT_ROOT_DIR="$( ${git}/bin/git rev-parse --show-toplevel; )"
      else
        echo "ERROR: Could not find root of the project."
        exit 1
      fi

      PROJECT_TESTS_DIR="$PROJECT_ROOT_DIR$PROJECT_TESTS_DIR"
      PROJECT_PATH="$PROJECT_ROOT_DIR/cli/target/debug";
      PROJECT_PATH="$PROJECT_PATH:$PROJECT_ROOT_DIR/pkgdb/bin";
    fi
    export PROJECT_TESTS_DIR

    # TODO: we shouldn't do this but rather use absolute paths
    # Look if we can use https://github.com/abathur/resholve
    export PATH="$PROJECT_PATH${lib.makeBinPath paths}"

    # Copy PROJECT_TESTS_DIR to temporary directory
    WORKDIR=$(mktemp -d -t ${PROJECT_NAME}-XXXXXX)
    cp -R $PROJECT_TESTS_DIR/* $WORKDIR
    cd $WORKDIR

    # Declare project specific dependencies
    ${
      if PKGDB_BIN == null
      then "export PKGDB_BIN='pkgdb';"
      else "export PKGDB_BIN='${PKGDB_BIN}';"
    }
    ${
      if PKGDB_IS_SQLITE3_BIN == null
      then "export PKGDB_IS_SQLITE3_BIN=\"$PROJECT_TESTS_DIR/is_sqlite3\";"
      else "export PKGDB_IS_SQLITE3_BIN='${PKGDB_IS_SQLITE3_BIN}';"
    }
    ${
      if PKGDB_SEARCH_PARAMS_BIN == null
      then
        "export PKGDB_SEARCH_PARAMS_BIN="
        + "\"$PROJECT_TESTS_DIR/search-params\";"
      else "export PKGDB_SEARCH_PARAMS_BIN='${PKGDB_SEARCH_PARAMS_BIN}';"
    }


    usage() {
          cat << EOF
    Usage: $0 [--pkgdb <PKGDB BINARY>| -P <PKGDB BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -P, --pkgdb       Path to pkgdb binary (Default: $PKGDB_BIN)
        -T, --tests       Path to folder of tests (Default: $PROJECT_TESTS_DIR)
        -W, --watch       Run tests in a continuous watch mode
        -h, --help        Prints help information
    EOF
    }

    WATCH=;
    declare -a _FLOX_TESTS;
    _FLOX_TESTS=();
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        -[pP]|--pkgdb)        export PKGDB_BIN="''${2?}"; shift; ;;
        -[tT]|--tests)        export TESTS_DIR="''${2?}"; shift; ;;
        -[wW]|--watch)        WATCH=:; ;;
        -h|--help|-u|--usage) usage; exit 0; ;;
        --)                   shift; break; ;;
        *)
          if [[ -e "$1" ]]; then
            _FLOX_TESTS+=( "$1" );
          else
            echo "''${0##*/} ERROR: Unrecognized arg(s) '$*'" >&2;
            usage;
            exit 1;
          fi
        ;;
      esac
      shift;
    done

    # Default flag values
    : "''${TESTS_DIR:=$WORKDIR}";
    export TESTS_DIR;

    if [[ "''${#_FLOX_TESTS[@]}" -lt 1 ]]; then
      _FLOX_TESTS=( "$TESTS_DIR" );
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
      echo "  PKGDB_BIN:                $PKGDB_BIN";
      echo "  PKGDB_ISSQLITE3_BIN:      $PKGDB_IS_SQLITE3_BIN";
      echo "  PKGDB_SEARCH_PARAMS_BIN:  $PKGDB_SEARCH_PARAMS_BIN";
      echo "  PROJECT_TESTS_DIR:        $PROJECT_TESTS_DIR";
      echo "  tests:                    ''${_FLOX_TESTS[*]}";
      echo "  bats                      ${batsWith}/bin/bats";
      echo "  bats options              ''${_BATS_ARGS[*]}";
      echo "  bats tests                ''${_FLOX_TESTS[*]}";
    } >&2;

    # Run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$PKGDB_BIN"                           \
           "$PKGDB_IS_SQLITE3_BIN" "$PKGDB_SEARCH_PARAMS_BIN"  \
        |${entr}/bin/entr -s "bats ''${_BATS_ARGS[*]} ''${_FLOX_TESTS[*]}";
    else
      exec -a "$0" ${batsWith}/bin/bats "''${_BATS_ARGS[@]}"    \
                                        "''${_FLOX_TESTS[@]}";
    fi
  ''
