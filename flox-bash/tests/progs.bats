#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test runtime dependencies of `flox' are resolved to `/nix/store' paths.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

destroy_envs() {
  "$FLOX_CLI" destroy -e "$TEST_ENVIRONMENT" --origin -f||:;
}

setup_file() {
  common_setup;
  TEST_ENVIRONMENT='_testing_progs';
  destroy_envs;
}

teardown_file() {
  destroy_envs;
}


# ---------------------------------------------------------------------------- #

# Run a command in the context of `flox-bash' after it has processed `utils.sh'.
# This file handles resolution of runtime dependencies, so we only care about
# testing past that point of initialization.
util() {
  # Perform a minimal form of `flox-bash/lib/init.sh' required to support
  # using internal `flox-bash/lib/utils.sh' routines.
  _prefix="$FLOX_PACKAGE";
  _lib="$_prefix/lib";
  _libexec="$_prefix/libexec";
  _etc="$_prefix/etc";

  # push current options
  _old_opts="$( shopt -p; )";
  shopt -s extglob;
  shopt -s nullglob;
  _OLD_PATH="$PATH";
  PATH='/bin:/sbin:/usr/bin:/usr/local/bin'
  PATH="$PATH:/run/wrappers/bin:/run/current-system/sw/bin"

  # Run utils setup
  #shellcheck source-path=SCRIPTDIR
  #shellcheck source=../lib/utils.sh
  . "$_lib/utils.sh";

  # Run the given command and stash the exit code
  eval "$*";
  _ec="$?";

  # restore old options
  eval "$_old_opts";
  PATH="$_OLD_PATH";

  # Don't forget to use the exit code from our command.
  return "$_ec";
}

cmds=(
  ansifilter awk basename bash cat chmod cmp column cp curl cut dasel date
  dirname getent gh git grep gum id jq ln man mkdir mktemp mv nix nix-editor
  nix-store pwd readlink realpath rm rmdir sed sh sleep sort stat tail tar tee
  touch tr uname uuid xargs zgrep semver
);


# ---------------------------------------------------------------------------- #

@test "runtime dependencies in '/nix/store'" {
  for p in "${cmds[@]}"; do
    run util echo "\$_$p";
    assert_output --regexp "^/nix/store/.*/$p\$";
    run util echo "\$invoke_$p";
    assert_output --regexp "^invoke /nix/store/.*/$p\$";
  done
}


# ---------------------------------------------------------------------------- #

@test "ensure activated shell doesn't inherit '_${cmds[1]}'" {
  run "$FLOX_CLI" create -e "$TEST_ENVIRONMENT";
  assert_success;
  run "$FLOX_CLI" install -e "$TEST_ENVIRONMENT" hello bash;
  assert_success;
  run "$FLOX_CLI" activate -- bash -c "echo \"\${_${cmds[1]}:-NOPE}\";";
  assert_output --partial NOPE;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
