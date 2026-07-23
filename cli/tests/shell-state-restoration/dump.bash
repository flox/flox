#!/usr/bin/env bash
# bash state-dump helper for shell-state-restoration.bats.
#
# Snapshots bash shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single bash
# process so the diff catches anything activate/deactivate failed to
# restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR, _TEST_HARNESS_NOISE_RE
#
# Usage:
#   dump.bash <pre.txt> <post.txt>
#
# We dump *names*, not values, to avoid noise from values that
# legitimately change during the activate→deactivate cycle (e.g. PWD).
# Only true shell internals and host/harness noise are filtered here;
# intentional (allow-listed) leaks are left in and classified by
# _assert_state_restored in shell-state-restoration.bats, so the allow-list
# can't silently rot.

set +e
export FLOX_SHELL=$(command -v bash)

# Stable ordering across platforms — macOS's default locale and Linux's
# collate underscores differently, which swaps the position of names
# like BASHOPTS / BATS_LIB_PATH between sort runs.
export LC_ALL=C

# bash-internal volatile names and our own dump helpers, filtered from
# the variable list.
_BASH_INTERNAL_VAR_RE='^(BASH(PID|_LINENO|_SOURCE|_COMMAND|_ARGV|_ARGC|_REMATCH|_SUBSHELL)|FUNCNAME|LINENO|SECONDS|RANDOM|SRANDOM|_|PIPESTATUS|HISTCMD|EPOCHSECONDS|EPOCHREALTIME|COLUMNS|LINES|OPTIND|OPTARG|out|_BASH_INTERNAL_VAR_RE)$'

_flox_dump_state() {
  local out="$1"
  {
    echo '=== SET_OPTIONS ==='
    set -o | sort
    echo '=== SHOPT ==='
    shopt -p | sort
    echo '=== FUNCTIONS ==='
    # Names only (like the zsh/fish dumps): function *bodies* churn between
    # snapshots and the leak signal is whether a new function name survives
    # deactivate. _assert_state_restored classifies each leaked name.
    compgen -A function \
      | sort \
      | grep -vE "^(_flox_dump_state)\$"
    echo '=== VARIABLES ==='
    compgen -v \
      | sort -u \
      | grep -vE "$_BASH_INTERNAL_VAR_RE" \
      | grep -vE "$_TEST_HARNESS_NOISE_RE"
  } > "$out"
}

_flox_dump_state "$1"

eval "$($FLOX_BIN activate -d "$PROJECT_DIR")"
# `--print-script` takes the invocation type map that `activate` recorded
# in `_FLOX_INVOCATION_TYPES` (here: in-place). See deactivate.bats.
eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"

_flox_dump_state "$2"
