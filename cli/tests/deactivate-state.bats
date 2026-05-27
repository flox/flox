#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end state-restoration tests for `flox deactivate --print-script`.
#
# These tests snapshot all shell state (functions, options, unexported
# variables) before activation and after deactivation, then diff the two.
# They verify that activation/deactivation is fully reversible — nothing
# leaks past the deactivate boundary.
#
# Each shell has its own snapshot helper because the dump commands differ:
#   - bash: `declare -F` (function names), `declare -f` (bodies),
#           `compgen -v` (variable names, including unexported),
#           `set -o` + `shopt -p` (options).
#   - zsh:  `print -l ${(k)functions}` (names),
#           `print -l "${(@kv)functions}"` (bodies),
#           `typeset +` (variable names), `setopt` (options).
#   - fish: `functions -n` (names), `functions <name>` (bodies),
#           `set --names` (variable names with all scopes).
#   - tcsh: `alias` (aliases — tcsh has no functions),
#           `set` and `setenv` (variables — tcsh has no notion of
#           "unexported" beyond shell vs. env vars).
#
# Each helper writes a normalized, sorted snapshot to a file. The test
# diffs pre vs. post snapshots, filtering shell-internal noise (LINENO,
# RANDOM, BASH_COMMAND, history vars, etc.). A clean diff means full
# restoration.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=deactivate,deactivate:state

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

setup() {
  common_test_setup
  home_setup test
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"

  # The dump helpers need to run in a clean outer-activation context.
  # If a developer-shell auto-activation has leaked `_FLOX_HOOK_*` vars
  # into bats, activation will treat this as nested and skip the
  # snapshot/restore logic, hiding leaks the test is meant to catch.
  unset _FLOX_HOOK_SAVE_FPATH _FLOX_HOOK_SAVE_COMPINIT_DUMPFILE _FLOX_HOOK_DIFF
  unset _FLOX_SOURCED_PROFILE_SCRIPTS
}

teardown() {
  cat_teardown_fifo
  if [ -n "${PROJECT_DIR:-}" ]; then
    wait_for_activations "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Names that are intentionally allowed to differ between pre-activate
# and post-deactivate snapshots. Each entry has a one-line justification.
# Removing an entry here means we believe the leak should be cleaned up.
#
# Functions:
#   _flox_hook    — auto-activate prompt hook intentionally left
#                   registered so re-entry on `cd` still fires (DEV-86).
# Variables:
#   PROMPT_COMMAND — carries _flox_hook into the user's shell; tracked
#                    with the hook above.
#   _FLOX_SOURCED_PROFILE_SCRIPTS
#                  — tracking var, intentionally NOT updated on
#                    deactivate (see deactivate.bats:206).
#   _activate_d / _flox_activate_tracer / _flox_activations
#                  — set unconditionally on activate; see
#                    gen_rc/zsh.rs Action::Deactivate("TODO: we might
#                    not need to set these in the first place").
#                    Remove from allow-list once that TODO is resolved.
_ALLOWED_LEAKS_NAMES='_flox_hook|PROMPT_COMMAND|_FLOX_SOURCED_PROFILE_SCRIPTS|_activate_d|_flox_activate_tracer|_flox_activations'
_ALLOWED_LEAKS_RE="^(${_ALLOWED_LEAKS_NAMES})\$"

# bats test_tags=deactivate,deactivate:state:bash
@test "bash: deactivate restores pre-activation shell state" {
  project_setup

  pre="${BATS_TEST_TMPDIR}/pre.txt"
  post="${BATS_TEST_TMPDIR}/post.txt"

  # Bash dump helper. Captures:
  #   1. `set -o` options (errexit, pipefail, ...)
  #   2. `shopt -p` options (extglob, globstar, ...)
  #   3. Function names (sorted), then full bodies via `declare -f`.
  #   4. Variable names from `compgen -v`, filtered for shell-internal
  #      volatiles that change on every command (LINENO, RANDOM, _, ...).
  # We dump *names*, not values, to avoid noise from values that
  # legitimately change during the activate→deactivate cycle (e.g. PWD).
  # We also drop names matching $_ALLOWED_LEAKS_RE before snapshotting;
  # this keeps the diff sensitive to *new* leaks while ignoring the
  # ones already cleared as intentional.
  run bash -c "
    set +e
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\"\$(command -v bash)\"

    _flox_dump_state() {
      local out=\"\$1\"
      {
        echo '=== SET_OPTIONS ==='
        set -o | sort
        echo '=== SHOPT ==='
        shopt -p | sort
        echo '=== FUNCTIONS ==='
        compgen -A function \
          | sort \
          | grep -vE '^(_flox_dump_state|${_ALLOWED_LEAKS_NAMES})\$'
        echo '=== FUNCTION_BODIES ==='
        # declare -f dumps all functions; strip the dump helper and
        # allow-listed names (matched by 'NAME ()' header line).
        declare -f \
          | awk -v leaks='${_ALLOWED_LEAKS_NAMES}' '
              BEGIN{ skip=0 }
              /^_flox_dump_state \\(\\) *\$/ { skip=1; next }
              \$0 ~ \"^(\" leaks \") \\\\(\\\\) *\$\" { skip=1; next }
              skip && /^\\} *\$/ { skip=0; next }
              !skip
            '
        echo '=== VARIABLES ==='
        compgen -v \
          | sort -u \
          | grep -vE '^(BASH(PID|_LINENO|_SOURCE|_COMMAND|_ARGV|_ARGC|_REMATCH|_SUBSHELL)|FUNCNAME|LINENO|SECONDS|RANDOM|SRANDOM|_|PIPESTATUS|HISTCMD|EPOCHSECONDS|EPOCHREALTIME|COLUMNS|LINES|OPTIND|OPTARG|out)\$' \
          | grep -vE '${_ALLOWED_LEAKS_RE}'
      } > \"\$out\"
    }

    _flox_dump_state '$pre'

    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    eval \"\$($FLOX_BIN deactivate --print-script)\"

    _flox_dump_state '$post'
  "
  assert_success

  # The diff should be empty modulo $_ALLOWED_LEAKS_RE (already filtered
  # out of both snapshots). If non-empty, surface it inline.
  run diff -u "$pre" "$post"
  if [ "$status" -ne 0 ]; then
    {
      echo "Shell state was not restored after deactivate."
      echo "If the leak is intentional, add the name to the"
      echo "_ALLOWED_LEAKS_RE allow-list with a justification."
      echo
      diff -u "$pre" "$post"
    } >&2
    fail "bash state differs between pre-activate and post-deactivate"
  fi
}
