bats_load_library bats-support
bats_load_library bats-assert
bats_require_minimum_version 1.5.0

# Common setup routines are defined in a separate function so this process may
# be extended.
# To do so a test file may redefine `setup_file' and call `common_setup' before
# writing their extensions.
common_setup() {
  if ! command -v expect >/dev/null 2>&1; then
    echo "ERROR: expect library needs to be in PATH."
    return 1 
  fi

  if [[ -z $FLOX_CLI ]]; then
    echo "ERROR: FLOX_CLI (a path to the binary) needs to be declared."
    return 1 
  fi

  FLOX_PACKAGE="$( readlink -f "$FLOX_CLI")"
  export FLOX_PACKAGE="${FLOX_PACKAGE%/*/*}"

  export TEST_ENVIRONMENT=_testing_

  # Remove any vestiges of previous test runs.
  $FLOX_CLI destroy -e "$TEST_ENVIRONMENT" --origin -f || :

  NIX_SYSTEM="$(
    $FLOX_CLI nix eval --impure --expr builtins.currentSystem --raw
  )"
  export NIX_SYSTEM

  HELLO_LINK="$(mktemp)"
  HELLO_PACKAGE="$(
    $FLOX_CLI build hello --print-out-paths --out-link "$HELLO_LINK"
  )"
  HELLO_FIRST8="${HELLO_PACKAGE#"${NIX_STORE:-/nix/store}/"}"
  HELLO_FIRST8="${HELLO_FIRST8:0:8}"
  export HELLO_LINK HELLO_PACKAGE HELLO_FIRST8

  # Simulate pure bootstrapping environment. It is challenging to get
  # the nix, gh, and flox tools to all use the same set of defaults.
  export REAL_XDG_CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
  FLOX_TEST_HOME="$(mktemp -d)"
  export FLOX_TEST_HOME
  export XDG_CACHE_HOME="$FLOX_TEST_HOME/.cache"
  mkdir "$XDG_CACHE_HOME"
  ln -s ~/.cache/nix "$XDG_CACHE_HOME/nix"
  export XDG_DATA_HOME="$FLOX_TEST_HOME/.local/share"
  export XDG_CONFIG_HOME="$FLOX_TEST_HOME/.config"
  export FLOX_CACHE_HOME="$XDG_CACHE_HOME/flox"
  export FLOX_META="$FLOX_CACHE_HOME/meta"
  export FLOX_DATA_HOME="$XDG_DATA_HOME/flox"
  export FLOX_ENVIRONMENTS="$FLOX_DATA_HOME/environments"
  export FLOX_CONFIG_HOME="$XDG_CONFIG_HOME/flox"

  unset FLOX_PROMPT_ENVIRONMENTS
  unset FLOX_ACTIVE_ENVIRONMENTS

  # Weirdest thing, gh will *move* your gh creds to the XDG_CONFIG_HOME
  # if it finds them in your home directory. Doesn't ask permission, just
  # does it. That is *so* not the right thing to do. (visible with strace)
  # 1121700 renameat(AT_FDCWD, "/home/brantley/.config/gh", AT_FDCWD, "/tmp/nix-shell.dtE4l4/tmp.JD4ki0ZezY/.config/gh") = 0
  # The way to defeat this behavior is by defining GH_CONFIG_DIR.
  export REAL_GH_CONFIG_DIR="$REAL_XDG_CONFIG_HOME/gh"
  export GH_CONFIG_DIR="$XDG_CONFIG_HOME/gh"
  rm -f tests/out/foo tests/out/subdir/bla
  rmdir tests/out/subdir tests/out || :
  rm -f "$FLOX_CONFIG_HOME/"{gitconfig,nix.conf}
  if [[ -z "${TESTS_DIR:-}" ]]; then
    TESTS_DIR="$(realpath ./tests)"
    export TESTS_DIR
  fi

  # Assume that versions:
  # a) start with numbers
  # b) contain at least one dot
  # c) contain only numbers and dots
  #
  # Of course this isn't true in general, but we can adhere to this
  # convention for this set of unit tests.
  #
  # N.B.:
  # - do NOT include $VERSION_REGEX within quotes (eats backslashes)
  # - do NOT add '$' at the end to anchor the match at EOL (doesn't work)
  export VERSION_REGEX='[0-9]+\.[0-9.]+'
}

# Shared teardown process.
common_teardown() {
  rm -f "$HELLO_LINK"
}



# setup_file() function run once for a given bats test file.
# This function may be redefined by individual test files, but running
# `common_setup' is the recommended minimum.
setup_file() {
  common_setup
}

teardown_file() {
  common_teardown
}
