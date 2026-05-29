#!/usr/bin/env bash
# bash state-dump helper for deactivate-state.bats.
#
# Snapshots bash shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single bash
# process so the diff catches anything activate/deactivate failed to
# restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR
#   _ALLOWED_LEAKS_NAMES, _ALLOWED_LEAKS_RE, _TEST_HARNESS_NOISE_RE
#
# Usage:
#   dump.bash <pre.txt> <post.txt>
#
# We dump *names*, not values, to avoid noise from values that
# legitimately change during the activate→deactivate cycle (e.g. PWD).
# Names matching $_ALLOWED_LEAKS_RE are dropped before snapshotting;
# this keeps the diff sensitive to *new* leaks while ignoring the
# ones already cleared as intentional.

set +e
export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    compgen -A function \
      | sort \
      | grep -vE "^(_flox_dump_state|${_ALLOWED_LEAKS_NAMES})\$"
    echo '=== FUNCTION_BODIES ==='
    # declare -f dumps all functions; strip the dump helper and
    # allow-listed names (matched by 'NAME ()' header line).
    declare -f \
      | awk -v leaks="${_ALLOWED_LEAKS_NAMES}" '
          BEGIN{ skip=0 }
          /^_flox_dump_state \(\) *$/ { skip=1; next }
          $0 ~ "^(" leaks ") \\(\\) *$" { skip=1; next }
          skip && /^\} *$/ { skip=0; next }
          !skip
        '
    echo '=== VARIABLES ==='
    compgen -v \
      | sort -u \
      | grep -vE "$_BASH_INTERNAL_VAR_RE" \
      | grep -vE "$_ALLOWED_LEAKS_RE" \
      | grep -vE "$_TEST_HARNESS_NOISE_RE"
  } > "$out"
}

_flox_dump_state "$1"

eval "$($FLOX_BIN activate -d "$PROJECT_DIR")"
eval "$($FLOX_BIN deactivate --print-script)"

_flox_dump_state "$2"
