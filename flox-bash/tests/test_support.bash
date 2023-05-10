bats_load_library bats-support
bats_load_library bats-assert
bats_require_minimum_version 1.5.0

# setup_file() function run once for a given bats test file.
setup_file() {
  set -x

  if [ -z "$FLOX_CLI" ]; then
    if [ -L ./result ]; then
      FLOX_PACKAGE=$(readlink ./result)
    else
      FLOX_PACKAGE=$(flox build -A flox --print-out-paths --substituters "")
    fi
    export FLOX_PACKAGE
    export FLOX_CLI=$FLOX_PACKAGE/bin/flox
    export FLOX_PACKAGE_FIRST8=$(echo $FLOX_PACKAGE | dd bs=c skip=11 count=8 2>/dev/null)
  fi
  export FLOX_DISABLE_METRICS="true"
  # Remove any vestiges of previous test runs.
  $FLOX_CLI destroy -e $TEST_ENVIRONMENT --origin -f || :
  export TEST_ENVIRONMENT=_testing_
  export NIX_SYSTEM=$($FLOX_CLI nix --extra-experimental-features nix-command show-config | awk '/system = / {print $NF}')
  # Simulate pure bootstrapping environment. It is challenging to get
  # the nix, gh, and flox tools to all use the same set of defaults.
  export REAL_XDG_CONFIG_HOME=${XDG_CONFIG_HOME:-$HOME/.config}
  export FLOX_TEST_HOME=$(mktemp -d)
  export XDG_CACHE_HOME=$FLOX_TEST_HOME/.cache
  mkdir $XDG_CACHE_HOME
  ln -s ~/.cache/nix $XDG_CACHE_HOME/nix
  export XDG_DATA_HOME=$FLOX_TEST_HOME/.local/share
  export XDG_CONFIG_HOME=$FLOX_TEST_HOME/.config
  export FLOX_CACHE_HOME=$XDG_CACHE_HOME/flox
  export FLOX_META=$FLOX_CACHE_HOME/meta
  export FLOX_DATA_HOME=$XDG_DATA_HOME/flox
  export FLOX_ENVIRONMENTS=$FLOX_DATA_HOME/environments
  export FLOX_CONFIG_HOME=$XDG_CONFIG_HOME/flox

  unset FLOX_PROMPT_ENVIRONMENTS
  unset FLOX_ACTIVE_ENVIRONMENTS

  # Weirdest thing, gh will *move* your gh creds to the XDG_CONFIG_HOME
  # if it finds them in your home directory. Doesn't ask permission, just
  # does it. That is *so* not the right thing to do. (visible with strace)
  # 1121700 renameat(AT_FDCWD, "/home/brantley/.config/gh", AT_FDCWD, "/tmp/nix-shell.dtE4l4/tmp.JD4ki0ZezY/.config/gh") = 0
  # The way to defeat this behavior is by defining GH_CONFIG_DIR.
  export REAL_GH_CONFIG_DIR=$REAL_XDG_CONFIG_HOME/gh
  export GH_CONFIG_DIR=$XDG_CONFIG_HOME/gh
  rm -f tests/out/foo tests/out/subdir/bla
  rmdir tests/out/subdir tests/out || :
  rm -f $FLOX_CONFIG_HOME/{gitconfig,nix.conf}
  export TESTS_DIR=$(realpath ./tests)

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

  set +x
}
