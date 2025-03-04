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
  flox,
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
  PROJECT_NAME ? "flox-cli-tests",
  PROJECT_TESTS_DIR ? ./../../cli/tests,
  localDev ? false,
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

  # TODO: we shouldn't do this but rather use absolute paths
  # Look if we can use https://github.com/abathur/resholve
  export PATH="$PROJECT_PATH:${lib.makeBinPath paths}"

  # Copy PROJECT_TESTS_DIR to temporary directory
  WORKDIR="$( mktemp -d -t ${PROJECT_NAME}-XXXXXX; )";
  cp -RL "$PROJECT_TESTS_DIR/"* "$WORKDIR";
  cd "$WORKDIR"||exit;

  usage() {
        cat << EOF
  Usage: $0 [--tests <TESTS_DIR>| -T <TESTS_DIR>] \
            [--watch | -W] \
            [--help | -h] -- [BATS ARGUMENTS]

  Available options:
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

  export _FLOX_LOCAL_DEV="${builtins.toString localDev}"
  ${lib.optionalString (!localDev) ''
    # Override any local mutable paths set by the devShell.
    export GENERATED_DATA='${./../../test_data/generated}'
    export MANUALLY_GENERATED='${./../../test_data/manually_generated}'
    export INPUT_DATA='${./../../test_data/input_data}'

    export FLOX_BIN="${flox}/bin/flox"
    export NIX_BIN="${nix}/bin/nix"
    export BUILDENV_BIN="${flox-buildenv}/bin/buildenv"
    export NIX_PLUGINS="${flox-nix-plugins}/lib/nix-plugins"
    export WATCHDOG_BIN="${flox-watchdog}/libexec/flox-watchdog"
    export FLOX_ACTIVATIONS_BIN="${flox-activations}/bin/flox-activations"
    export PROCESS_COMPOSE_BIN='${process-compose}/bin/process-compose'
    export FLOX_INTERPRETER='${flox-interpreter}'
  ''}

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
    echo "  PROJECT_TESTS_DIR:        $PROJECT_TESTS_DIR";
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
