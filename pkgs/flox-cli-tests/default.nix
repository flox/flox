{
  gcc,
  runCommandCC,
  stdenv,
  lib,
  bashInteractive,
  fish,
  tcsh,
  zsh,
  dash,
  bats,
  coreutils,
  entr,
  expect,
  findutils,
  flox-pkgdb,
  flox-cli,
  gawk,
  git,
  gnugrep,
  gnupg,
  gnused,
  gnutar,
  jq,
  nix,
  yq,
  openssh,
  parallel,
  podman,
  unixtools,
  which,
  writeShellScriptBin,
  PROJECT_NAME ? "flox-cli-tests",
  PROJECT_TESTS_DIR ? ./../../cli/tests,
  NIX_BIN ? "${nix}/bin/nix",
  PKGDB_BIN ? "${flox-pkgdb}/bin/pkgdb",
  LD_FLOXLIB ? "${flox-pkgdb}/lib/ld-floxlib.so",
  FLOX_BIN ? "${flox-cli}/bin/flox",
}: let
  batsWith = bats.withLibraries (p: [
    p.bats-assert
    p.bats-file
    p.bats-support
  ]);

  paths =
    [
      bashInteractive
      fish
      tcsh
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
      openssh
      parallel
      unixtools.util-linux
      which
      yq
    ]
    # TODO: this hack is not going to be needed once we test against sutff on system
    ++ lib.optional stdenv.isDarwin (
      runCommandCC "locale" {
        source = ''
          #include <stdio.h>
          int main(){
            printf("UTF-8");
            return 0;
          }
        '';
        buildInputs = [gcc];
      } ''
        mkdir -p "$out/bin"
        echo "$source" | gcc -Wall -o "$out/bin/$name" -xc -
      ''
    )
    # Containerize tests need a container runtime.
    # Since we're building and building only works on linux,
    # we only include podman on linux.
    ++ lib.optionals stdenv.isLinux [
      podman
    ];
in
  # TODO: we should run tests against different shells
  writeShellScriptBin PROJECT_NAME ''
    set -eu;
    set -o pipefail;

    # Find root of the subproject if not specified
    PROJECT_TESTS_DIR='${PROJECT_TESTS_DIR}';
    # Find top level of the project
    PROJECT_PATH="";
    case "$PROJECT_TESTS_DIR" in
      /nix/store/*) :; ;;
      *)
        if ${git}/bin/git rev-parse --is-inside-work-tree >/dev/null 2>&1; then
          PROJECT_ROOT_DIR="$( ${git}/bin/git rev-parse --show-toplevel; )";
          export PROJECT_ROOT_DIR;
        else
          echo "ERROR: Could not find root of the project.";
          exit 1;
        fi
        PROJECT_TESTS_DIR="$PROJECT_ROOT_DIR$PROJECT_TESTS_DIR";
        PROJECT_PATH="$PROJECT_ROOT_DIR/cli/target/debug";
        PROJECT_PATH="$PROJECT_PATH:$PROJECT_ROOT_DIR/pkgdb/bin";
      ;;
    esac
    export PROJECT_TESTS_DIR;

    # TODO: we shouldn't do this but rather use absolute paths
    # Look if we can use https://github.com/abathur/resholve
    export PATH="$PROJECT_PATH:${lib.makeBinPath paths}"

    # Copy PROJECT_TESTS_DIR to temporary directory
    WORKDIR="$( mktemp -d -t ${PROJECT_NAME}-XXXXXX; )";
    cp -RL "$PROJECT_TESTS_DIR/"* "$WORKDIR";
    cd "$WORKDIR"||exit;

    # Declare project specific dependencies
    ${
      if NIX_BIN == null
      then "export NIX_BIN='nix';"
      else "export NIX_BIN='${NIX_BIN}';"
    }
    ${
      if PKGDB_BIN == null
      then ''export PKGDB_BIN="$(command -v pkgdb)";''
      else "export PKGDB_BIN='${PKGDB_BIN}';"
    }
    ${
      if LD_FLOXLIB == null
      then "export LD_FLOXLIB='ld-floxlib.so';"
      else "export LD_FLOXLIB='${LD_FLOXLIB}';"
    }
    ${
      if FLOX_BIN == null
      then "export FLOX_BIN='flox';"
      else "export FLOX_BIN='${FLOX_BIN}';"
    }

    usage() {
          cat << EOF
    Usage: $0 [--flox <FLOX BINARY>| -F <FLOX BINARY>] \
              [--pkgdb <PKGDB BINARY>| -P <PKGDB BINARY>] \
              [--nix <NIX BINARY>| -N <NIX BINARY>] \
              [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
              [--watch | -W] \
              [--help | -h] -- [BATS ARGUMENTS]

    Available options:
        -F, --flox          Path to flox binary (Default: $FLOX_BIN)
        -P, --pkgdb         Path to pkgdb binary (Default: $PKGDB_BIN)
        -L, --ld-floxlib    Path to ld-floxlib.so (Default: $LD_FLOXLIB)
        -N, --nix           Path to nix binary (Default: $NIX_BIN)
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
        -[fF]|--flox)         export FLOX_BIN="''${2?}"; shift; ;;
        -[pP]|--pkgdb)        export PKGDB_BIN="''${2?}"; shift; ;;
        -[lL]|--ld-floxlib)   export LD_FLOXLIB="''${2?}"; shift; ;;
        -[nN]|--nix)          export NIX_BIN="''${2?}"; shift; ;;
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
      echo "  FLOX_BIN:                 $FLOX_BIN";
      echo "  PKGDB_BIN:                $PKGDB_BIN";
      echo "  NIX_BIN:                  $NIX_BIN";
      echo "  LD_FLOXLIB:               $LD_FLOXLIB";
      echo "  PROJECT_TESTS_DIR:        $PROJECT_TESTS_DIR";
      echo "  bats                      ${batsWith}/bin/bats";
      echo "  bats options              ''${_BATS_ARGS[*]}";
      echo "  bats tests                ''${_FLOX_TESTS[*]}";
    } >&2;

    # Run basts either via entr or just a single run
    if [[ -n "''${WATCH:-}" ]]; then
      find "$TESTS_DIR" "$NIX_BIN" "$PKGDB_BIN" "$LD_FLOXLIB" "$FLOX_BIN"    \
        |${entr}/bin/entr -s "bats ''${_BATS_ARGS[*]} ''${_FLOX_TESTS[*]}";
    else
      exec -a "$0" ${batsWith}/bin/bats "''${_BATS_ARGS[@]}"    \
                                        "''${_FLOX_TESTS[@]}";
    fi
  ''
