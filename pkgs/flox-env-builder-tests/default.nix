{
  self,
  lib,
  bats,
  entr,
  coreutils,
  findutils,
  flox-env-builder,
  flox-pkgdb,
  git,
  jq,
  nix,
  writeShellScriptBin,
  PROJECT_NAME ? "flox-env-builder-tests",
  PROJECT_TESTS_DIR ? ./../../env-builder/tests,
  PKGDB_BIN ? "${flox-pkgdb}/bin/pkgdb",
  ENV_BUILDER_BIN ? "${flox-env-builder}/bin/flox-env-builder",
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths = [
    batsWith
    coreutils
    findutils
    git
    jq
    nix
  ];
in
  # TODO: we should run tests against different shells
  writeShellScriptBin PROJECT_NAME ''
    set -euo pipefail

    # Find top level of the project
    if ${git}/bin/git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
      export PROJECT_ROOT_DIR="$( ${git}/bin/git rev-parse --show-toplevel; )"
    else
      echo "ERROR: Could not find root of the project."
      exit 1
    fi

    # Find root of the subproject if not specified
    PROJECT_TESTS_DIR=${PROJECT_TESTS_DIR}
    PROJECT_PATH=""
    if [[ $PROJECT_TESTS_DIR != "/nix/store/"* ]]; then
      PROJECT_TESTS_DIR="$PROJECT_ROOT_DIR$PROJECT_TESTS_DIR"
      PROJECT_PATH="$PROJECT_ROOT_DIR/target/debug:$PROJECT_ROOT_DIR/pkgdb/bin:$PROJECT_ROOT_DIR/env-builder/bin:"
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
      if ENV_BUILDER_BIN == null
      then "export ENV_BUILDER_BIN='flox-env-builder';"
      else "export ENV_BUILDER_BIN='${ENV_BUILDER_BIN}';"
    }

    # Generate lockfiles for tests
    pushd $WORKDIR
      chmod -R +w ./fixtures/lockfiles
      for i in $(find ./ -name "*.toml"); do
        $PKGDB_BIN manifest lock --ga-registry "$i" | jq > "$(dirname $i)/$(basename $i toml)lock"
      done
    popd



    usage() {
          cat << EOF
    Usage: $0 [--env-builder <ENV_BUILDER BINARY>| -E <ENV_BUILDER BINARY>] \
              [--pkgdb <PKGDB BINARY>| -P <PKGDB BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -E, --env-builder   Path to env-builder binary (Default: $ENV_BUILDER_BIN)
        -P, --pkgdb         Path to pkgdb binary (Default: $PKGDB_BIN)
        -T, --tests         Path to folder of tests (Default: $PROJECT_TESTS_DIR)
        -W, --watch         Run tests in a continuous watch mode
        -h, --help          Prints help information
    EOF
    }


    WATCH=;
    declare -a _FLOX_TESTS;
    _FLOX_TESTS=();
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        -[eE]|--env-builder)  export ENV_BUILDER_BIN="''${2?}"; shift; ;;
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
    : "''${TESTS_DIR:=$PROJECT_TESTS_DIR}";
    export TESTS_DIR;

    # Default flag values
    : "''${TESTS_DIR:=$PROJECT_TESTS_DIR}";
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
      echo "  ENV_BUILDER_BIN:          $ENV_BUILDER_BIN";
      echo "  PKGDB_BIN:                $PKGDB_BIN";
      echo "  PROJECT_TESTS_DIR:        $PROJECT_TESTS_DIR";
      echo "  bats                      ${batsWith}/bin/bats";
      echo "  bats options              ''${_BATS_ARGS[*]}";
      echo "  bats tests                ''${_FLOX_TESTS[*]}";
    } >&2;

    # Run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$PKGDB"  \
        |${entr}/bin/entr -s "bats ''${_BATS_ARGS[*]} ''${_FLOX_TESTS[*]}";
    else
      exec -a "$0" ${batsWith}/bin/bats "''${_BATS_ARGS[@]}" "''${_FLOX_TESTS[@]}";
    fi
  ''
