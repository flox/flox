#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test runtime dependencies of `flox' are resolved to `/nix/store' paths.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=progs, dependencies, init


# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;

  # Perform a minimal form of `flox-bash/lib/init.sh' required to support
  # using internal `flox-bash/lib/utils.sh' routines.
  _prefix="$( $FLOX_CLI --bash-passthru --prefix; )";
  _lib="$_prefix/lib";
  _libexec="$_prefix/libexec";
  _etc="$_prefix/etc";

  # Used to reset `PATH' to conventional UNIX system default.
  # This ensures that the `PATH' used by the test environment does not pollute
  # our results.
  _progs_PATH='/bin:/sbin:/usr/bin:/usr/local/bin';
  _progs_PATH="$_progs_PATH:/run/wrappers/bin:/run/current-system/sw/bin";

  export _prefix _lib _libexec _etc _progs_PATH;
}


# ---------------------------------------------------------------------------- #

# Run a command in the context of `flox-bash' after it has processed `utils.sh'.
# This file handles resolution of runtime dependencies, so we only care about
# testing past that point of initialization.
util() {
  # push current options
  _old_opts="$( shopt -p; )";
  shopt -s extglob;
  shopt -s nullglob;
  _OLD_PATH="$PATH";
  export PATH="$_progs_PATH";

  # Run utils setup
  #shellcheck source-path=SCRIPTDIR
  #shellcheck source=../flox-bash/lib/utils.sh
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
  'nix-store' pwd readlink realpath rm rmdir sed sh sleep sort stat tail tar tee
  touch tr uname uuid xargs zgrep semver 'parser-util'
);


# ---------------------------------------------------------------------------- #

@test "runtime dependencies in '/nix/store'" {
  for p in "${cmds[@]}"; do
    _p="${p//-/_}";
    run util echo "\$_$_p";
    assert_output --regexp "^/nix/store/.*/$p\$";
    run util echo "\$invoke_$_p";
    assert_output --regexp "^invoke /nix/store/.*/$p\$";
  done
}


# ---------------------------------------------------------------------------- #

@test "ensure activated shell doesn't inherit '_${cmds[1]}'" {
  run "$FLOX_CLI" --bash-passthru create -e "$TEST_ENVIRONMENT";
  assert_success;
  run "$FLOX_CLI" --bash-passthru install -e "$TEST_ENVIRONMENT" hello bash;
  assert_success;
  run "$FLOX_CLI" --bash-passthru activate -e "$TEST_ENVIRONMENT" --  \
        bash -c "echo \"\${_${cmds[1]}:-NOPE}\";";
  assert_output --partial NOPE;
}


# ---------------------------------------------------------------------------- #

# TODO: this can be removed when we have integrated `parser-util' into some
# commands, but for now we just want to be sure that it's callable.
@test "ensure 'parser-util' can be run" {
  run util "\$_parser_util --help";
  assert_success;
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
