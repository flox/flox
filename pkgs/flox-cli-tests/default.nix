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
  curl,
  diffutils,
  entr,
  expect,
  findutils,
  flox-activations,
  flox-buildenv,
  flox-nix-plugins,
  flox-watchdog,
  flox-cli,
  flox-interpreter,
  gawk,
  git,
  gnugrep,
  gnupg,
  gnused,
  gnutar,
  jq,
  man,
  nix,
  ncurses,
  yq,
  openssh,
  parallel,
  podman,
  procps,
  pstree,
  unixtools,
  util-linux,
  which,
  writeShellScriptBin,
  process-compose,
  GENERATED_DATA ? ./../../test_data/generated,
  MANUALLY_GENERATED ? ./../../test_data/manually_generated,
  INPUT_DATA ? ./../../test_data/input_data,
  PROJECT_NAME ? "flox-cli-tests",
  PROJECT_TESTS_DIR ? ./../../cli/tests,
  NIX_BIN ? "${nix}/bin/nix",
  BUILDENV_BIN ? "${flox-buildenv}/bin/buildenv",
  NIX_PLUGINS ? "${flox-nix-plugins}/lib/nix-plugins",
  FLOX_BIN ? "${flox-cli}/bin/flox",
  WATCHDOG_BIN ? "${flox-watchdog}/libexec/flox-watchdog",
  FLOX_ACTIVATIONS_BIN ? "${flox-activations}/bin/flox-activations",
}:
let
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
      curl
      diffutils
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
      man
      ncurses
      nix
      openssh
      parallel
      pstree
      unixtools.util-linux
      util-linux # for setsid
      which
      yq
      process-compose
      procps
      (podman.override (prev: {
        extraPackages = [ "/run/wrappers" ];
      }))
      "/run/wrappers"
    ]
    # TODO: this hack is not going to be needed once we test against stuff on system
    ++ lib.optional stdenv.isDarwin (
      runCommandCC "locale"
        {
          source = ''
            #include <stdio.h>
            int main(){
              printf("UTF-8");
              return 0;
            }
          '';
          buildInputs = [ gcc ];
        }
        ''
          mkdir -p "$out/bin"
          echo "$source" | gcc -Wall -o "$out/bin/$name" -xc -
        ''
    );
in
# TODO: we should run tests against different shells
writeShellScriptBin PROJECT_NAME ''
  set -eu;
  set -o pipefail;

  # Set the test data location
  export GENERATED_DATA='${GENERATED_DATA}'
  export MANUALLY_GENERATED='${MANUALLY_GENERATED}'
  export INPUT_DATA='${INPUT_DATA}'

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
    ;;
  esac
  export PROJECT_TESTS_DIR;

  # Find the latest released version of flox.
  # Despite the name this is generally _older_ than the tested version of flox.
  FLOX_LATEST_VERSION=${builtins.readFile ../../VERSION}
  export FLOX_LATEST_VERSION

  # if FLOX_VERSION is not set, use the latest released version
  # otherwise use the provided version
  # when running tests with just, just will set FLOX_VERSION.
  export FLOX_VERSION="''${FLOX_VERSION:-$FLOX_LATEST_VERSION}"

  # TODO: we shouldn't do this but rather use absolute paths
  # Look if we can use https://github.com/abathur/resholve
  export PATH="$PROJECT_PATH:${lib.makeBinPath paths}"

  # Copy PROJECT_TESTS_DIR to temporary directory
  WORKDIR="$( mktemp -d -t ${PROJECT_NAME}-XXXXXX; )";
  cp -RL "$PROJECT_TESTS_DIR/"* "$WORKDIR";
  cd "$WORKDIR"||exit;

  # Declare project specific dependencies
  ${if NIX_BIN == null then "export NIX_BIN='nix';" else "export NIX_BIN='${NIX_BIN}';"}
  ${
    if BUILDENV_BIN == null then
      ''export BUILDENV_BIN="$PROJECT_ROOT_DIR/build/flox-buildenv/bin/buildenv";''
    else
      "export BUILDENV_BIN='${BUILDENV_BIN}';"
  }
  ${
    if NIX_PLUGINS == null then
      ''export NIX_PLUGINS="$PROJECT_ROOT_DIR/build/nix-plugins/lib/nix-plugins";''
    else
      "export NIX_PLUGINS='${NIX_PLUGINS}';"
  }
  ${
    if WATCHDOG_BIN == null then
      # We pass this to daemonize which requires an absolute path
      "export WATCHDOG_BIN=\"$(command -v flox-watchdog)\";"
    else
      "export WATCHDOG_BIN='${WATCHDOG_BIN}';"
  }
  ${
    if FLOX_ACTIVATIONS_BIN == null then
      ''export FLOX_ACTIVATIONS_BIN="$(command -v flox-activations)";''
    else
      "export FLOX_ACTIVATIONS_BIN='${FLOX_ACTIVATIONS_BIN}';"
  }
  # TODO: we should probably make this an absolute path to avoid having to call
  # which "$FLOX_BIN" in user_dotfiles_setup
  ${if FLOX_BIN == null then "export FLOX_BIN='flox';" else "export FLOX_BIN='${FLOX_BIN}';"}
  export PROCESS_COMPOSE_BIN='${process-compose}/bin/process-compose';
  ${
    if flox-interpreter == null then
      ''export FLOX_INTERPRETER="$PROJECT_ROOT_DIR/build/flox-interpreter";''
    else
      ''export FLOX_INTERPRETER='${flox-interpreter}';''
  }

  usage() {
        cat << EOF
  Usage: $0 [--flox <FLOX BINARY>| -F <FLOX BINARY>] \
            [--watchdog <WATCHDOG BINARY | -K <WATCHDOG BINARY>] \
            [--flox-activations <FLOX ACTIVATIONS BINARY>] \
            [--nix-plugins <PLUGINS DIR>| -P <PLUGINS DIR>] \
            [--nix <NIX BINARY>| -N <NIX BINARY>] \
            [--input-data <INPUT DATA> | -I <INPUT DATA>] \
            [--generated-data <GENERATED DATA> | -G <GENERATED DATA>] \
            [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
            [--watch | -W] \
            [--help | -h] -- [BATS ARGUMENTS]

  Available options:
      -F, --flox           Path to flox binary (Default: $FLOX_BIN)
      -K, --watchdog       Path to the watchdog binary (Default: $WATCHDOG_BIN)
      -B, --buildenv       Path to buildenv binary (Default: $BUILDENV_BIN)
      -P, --nix-plugins    Path to dir with flox nix-plugins (Default: $NIX_PLUGINS)
      -N, --nix            Path to nix binary (Default: $NIX_BIN)
      -I, --input-data     Path to the input data directory (Default: $INPUT_DATA)
      -G, --generated-data Path to the generated data directory (Default: $GENERATED_DATA)
      -T, --tests          Path to folder of tests (Default: $PROJECT_TESTS_DIR)
      -c, --ci-runner      Which runner this job is on, if any
      -W, --watch          Run tests in a continuous watch mode
      -h, --help           Prints help information
  EOF
  }


  WATCH=;
  declare -a _FLOX_TESTS;
  _FLOX_TESTS=();
  while [[ "$#" -gt 0 ]]; do
    case "$1" in
      -[fF]|--flox)           export FLOX_BIN="''${2?}"; shift; ;;
      -[kK]|--watchdog)       export WATCHDOG_BIN="''${2?}"; shift; ;;
      -[bB]|--buildenv)       export BUILDENV_BIN="''${2?}"; shift; ;;
      --flox-activations)     export FLOX_ACTIVATIONS_BIN="''${2?}"; shift; ;;
      -[pP]|--nix-plugins)    export NIX_PLUGINS="''${2?}"; shift; ;;
      -[nN]|--nix)            export NIX_BIN="''${2?}"; shift; ;;
      -[iI]|--input-data)     export INPUT_DATA="''${2?}"; shift; ;;
      -[gG]|--generated-data) export GENERATED_DATA="''${2?}"; shift; ;;
      -[tT]|--tests)          export TESTS_DIR="''${2?}"; shift; ;;
      -[cC]|--ci-runner)      export FLOX_CI_RUNNER="''${2?}"; shift; ;;
      -[wW]|--watch)          WATCH=:; ;;
      -h|--help|-u|--usage)   usage; exit 0; ;;
      --)                     shift; break; ;;
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

  # Set the test data location
  export GENERATED_DATA=''${GENERATED_DATA:-'${GENERATED_DATA}'}
  export INPUT_DATA=''${INPUT_DATA:-'${INPUT_DATA}'}

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
  );

  flox_test_jobs="''${FLOX_TEST_JOBS:-4}"
  if [[ "$flox_test_jobs" -gt 2 ]]; then
    _BATS_ARGS+=( '--jobs' "$flox_test_jobs" )
    _BATS_ARGS+=( '--no-parallelize-across-files' )
  fi

  # Add the rest of the arguments provided to the script
  _BATS_ARGS+=("''${@:-}")

  {
    echo "''${0##*/}: Running test suite with:";
    echo "  FLOX_BIN:                 $FLOX_BIN";
    echo "  WATCHDOG_BIN:             $WATCHDOG_BIN";
    echo "  BUILDENV_BIN:             $BUILDENV_BIN";
    echo "  FLOX_ACTIVATIONS_BIN:     $FLOX_ACTIVATIONS_BIN";
    echo "  NIX_PLUGINS:                $NIX_PLUGINS";
    echo "  NIX_BIN:                  $NIX_BIN";
    echo "  FLOX_INTERPRETER:         $FLOX_INTERPRETER";
    echo "  PROJECT_TESTS_DIR:        $PROJECT_TESTS_DIR";
    echo "  INPUT_DATA:               $INPUT_DATA";
    echo "  GENERATED_DATA:           $GENERATED_DATA";
    echo "  bats                      ${batsWith}/bin/bats";
    echo "  bats options              ''${_BATS_ARGS[*]}";
    echo "  bats tests                ''${_FLOX_TESTS[*]}";
  } >&2;

  # Run basts either via entr or just a single run
  if [[ -n "''${WATCH:-}" ]]; then
    find "$TESTS_DIR" "$NIX_BIN" "$BUILDENV_BIN" "$NIX_PLUGINS" "$WATCHDOG_BIN" "$FLOX_BIN"    \
      |${entr}/bin/entr -s "bats ''${_BATS_ARGS[*]} ''${_FLOX_TESTS[*]}";
  else
    ${batsWith}/bin/bats "''${_BATS_ARGS[@]}"    \
                         "''${_FLOX_TESTS[@]}";
  fi
''
