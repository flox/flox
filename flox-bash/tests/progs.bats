#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test runtime dependencies of `flox' are resolved to `/nix/store' paths.
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

setup_file() {
  common_setup;
}

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
  if [[ -r "$_lib/progs.sh" ]]; then
    . "$_lib/progs.sh";
  fi
  . "$_lib/utils.sh";
  eval "$@";
  _ec="$?";

  # restore old options
  eval "$_old_opts";
  PATH="$_OLD_PATH";
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
#
#
#
# ============================================================================ #
