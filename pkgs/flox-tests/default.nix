{
  lib,
  bash,
  zsh,
  dash,
  bats,
  coreutils,
  entr,
  expect,
  findutils,
  flox,
  flox-bash,
  gawk,
  git,
  gnugrep,
  gnupg,
  gnused,
  gnutar,
  jq,
  nix-serve,
  openssh,
  parallel,
  unixtools,
  which,
  writeShellScriptBin,
  testsDir ? "/tests",
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
    gnupg
    gnused
    gnutar
    jq
    nix-serve
    openssh
    parallel
    unixtools.util-linux
    which
  ];
in
  writeShellScriptBin "flox-tests" ''

    export PATH="${lib.makeBinPath paths}"
    export PKGDB_BIN="${flox.PKGDB_BIN}"

    usage() {
          cat << EOF
    Usage: $0 [--flox <FLOX BINARY>| -F <FLOX BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -F, --flox          Path to flox binary (Default: flox)
        -T, --tests         Path to folder of tests (Default: $PWD${testsDir})
        -W, --watch         Run tests in a continuous watch mode
        -h, --help          Prints help information
    EOF
    }

    WATCH=
    declare -a _TESTS;
    _TESTS=();
    while [[ "$#" -gt 0 ]]; do
      case "$1" in
        -[fF]|--flox)         export FLOX_CLI="''${2?}"; shift; ;;
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

    if [[ -z "''${FLOX_CLI:-}" ]]; then
      if [[ -x "$PWD/target/debug/flox" ]]; then
        FLOX_CLI="$PWD/target/debug/flox";
      elif [[ -x "$PWD/target/release/flox" ]]; then
        FLOX_CLI="$PWD/target/release/flox";
      elif [[ -x "$PWD/result/bin/flox" ]]; then
        FLOX_CLI="$( readlink -f $PWD/result; )/bin/flox";
      elif command -v flox &> /dev/null; then
        echo "''${0##*/} WARNING: using flox executable from PATH" >&2;
        FLOX_CLI="$( command -v flox; )";
      fi
      export FLOX_CLI;
    fi

    export NIX_BIN="${flox-bash}/libexec/flox/nix";

    # Default flag values
    : "''${TESTS_DIR:=$PWD${testsDir}}";
    export TESTS_DIR FLOX_CLI;

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
      echo "  FLOX_CLI:     $FLOX_CLI";
      echo "  TESTS_DIR:    $TESTS_DIR";
      echo "  tests:        ''${_TESTS[*]}";
      echo "  bats options: ''${_BATS_ARGS[*]}";
      echo "  bats command: bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    } >&2;

    # run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$FLOX_CLI"  \
        |entr -s "bats ''${_BATS_ARGS[*]} ''${_TESTS[*]}";
    else
      exec -a "$0" bats "''${_BATS_ARGS[@]}" "''${_TESTS[@]}";
    fi
  ''
