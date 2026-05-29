#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end state-restoration test for `flox deactivate --print-script`.
#
# Snapshots all bash shell state (functions, options, unexported variables)
# before activation and after deactivation, then diffs the two. Verifies
# activation/deactivation is fully reversible — nothing leaks past the
# deactivate boundary.
#
# Bash-only for now. The dump primitives used here (`compgen -A function`,
# `declare -f`, `compgen -v`, `set -o`, `shopt -p`) are bash-specific.
# Equivalents exist in zsh (`print -l ${(k)functions}`, `typeset +`,
# `setopt`), fish (`functions -n`, `set --names`), and tcsh (`alias`,
# `set` / `setenv`); adding those shells is follow-up work — see
# https://linear.app/flox/issue/DEV-86.
#
# The snapshot writes a normalized, sorted view (LC_ALL=C) to a file. The
# test diffs pre vs. post, filtering shell-internal noise (LINENO, RANDOM,
# BASH_COMMAND, history vars, etc.) and the test harness's own setup vars.
# A clean diff means full restoration.
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
export _ALLOWED_LEAKS_NAMES='_flox_hook|PROMPT_COMMAND|_FLOX_SOURCED_PROFILE_SCRIPTS|_activate_d|_flox_activate_tracer|_flox_activations'
export _ALLOWED_LEAKS_RE="^(${_ALLOWED_LEAKS_NAMES})\$"

# Names that come from the test harness or the host OS, not from
# activate/deactivate. Filtered out of both snapshots so the diff
# only surfaces state that flox itself touched.
#   __FT_RAN_.*                   — flox-test setup-once flags
#                                   (HOME_SETUP, GITCONFIG_SETUP, …)
#   _FLOX_LOCAL_DEV               — bats / dev-shell signal
#   _FLOX_TEST_SUITE_MODE         — bats signal
#   _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS
#                                 — bats signal
#   _FLOX_USE_CATALOG_MOCK        — bats catalog mock pointer
#   __CF_USER_TEXT_ENCODING       — macOS process attribute
#   __NIX_DARWIN_SET_ENVIRONMENT_DONE
#                                 — nix-darwin marker on macOS runners
#   PATH_LOCALE                   — set by the activated env's locale
#                                   archive on macOS; orthogonal to the
#                                   activate/deactivate lifecycle this
#                                   test guards.
#   NIX_SSL_CERT_FILE             — set by the activated env's CA-cert
#                                   bundle on Linux; same orthogonality
#                                   as PATH_LOCALE.
export _TEST_HARNESS_NOISE_RE='^(__FT_RAN_.*|_FLOX_LOCAL_DEV|_FLOX_TEST_SUITE_MODE|_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS|_FLOX_USE_CATALOG_MOCK|__CF_USER_TEXT_ENCODING|__NIX_DARWIN_SET_ENVIRONMENT_DONE|PATH_LOCALE|NIX_SSL_CERT_FILE)$'

# Shared assertion: fail with the diff inline if pre and post snapshots
# differ. Caller passes a label naming the shell whose state was checked.
_assert_state_restored() {
  local shell="$1" pre="$2" post="$3"
  run diff -u "$pre" "$post"
  if [ "$status" -ne 0 ]; then
    {
      echo "Shell state was not restored after deactivate."
      echo "If the leak is intentional, add the name to the"
      echo "_ALLOWED_LEAKS_RE allow-list with a justification."
      echo
      echo "$output"
    } >&2
    fail "$shell state differs between pre-activate and post-deactivate"
  fi
}

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
    # Stable ordering across platforms — macOS's default locale and Linux's
    # collate underscores differently, which swaps the position of names
    # like BASHOPTS / BATS_LIB_PATH between sort runs.
    export LC_ALL=C

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
          | grep -vE '${_ALLOWED_LEAKS_RE}' \
          | grep -vE '${_TEST_HARNESS_NOISE_RE}'
      } > \"\$out\"
    }

    _flox_dump_state '$pre'

    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    eval \"\$($FLOX_BIN deactivate --print-script)\"

    _flox_dump_state '$post'
  "
  assert_success

  _assert_state_restored bash "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,deactivate:state:zsh
@test "zsh: deactivate restores pre-activation shell state" {
  project_setup

  pre="${BATS_TEST_TMPDIR}/pre.txt"
  post="${BATS_TEST_TMPDIR}/post.txt"

  # zsh dump primitives differ from bash:
  #   - `setopt`                 → set options (errexit, pipefail, …)
  #   - `print -l ${(ko)functions}` / `functions NAME`
  #                              → function names (sorted) and bodies
  #   - `print -l ${(ko)parameters}`
  #                              → variable names (sorted), including
  #                                non-exported locals.
  # The actual filter/dump lives in dump.zsh; see comments there.
  run zsh "$TESTS_DIR/deactivate-state/dump.zsh" "$pre" "$post"
  assert_success

  _assert_state_restored zsh "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,deactivate:state:fish
@test "fish: deactivate restores pre-activation shell state" {
  project_setup

  pre="${BATS_TEST_TMPDIR}/pre.txt"
  post="${BATS_TEST_TMPDIR}/post.txt"

  # fish dump primitives:
  #   - `functions --names`     → function names (comma-separated;
  #                               split with `string split`)
  #   - `functions NAME`        → function body
  #   - `set --names`           → variable names across all scopes
  # fish has no shopt/setopt equivalent. See dump.fish for details.
  run fish "$TESTS_DIR/deactivate-state/dump.fish" "$pre" "$post"
  assert_success

  _assert_state_restored fish "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,deactivate:state:tcsh
@test "tcsh: deactivate restores pre-activation shell state" {
  project_setup

  pre="${BATS_TEST_TMPDIR}/pre.txt"
  post="${BATS_TEST_TMPDIR}/post.txt"

  # tcsh dump shape differs from the other shells:
  #   - `alias`                 → alias definitions (csh has no
  #                               functions)
  #   - `set`                   → shell-variable names (awk extracts
  #                               the first whitespace field)
  #   - `setenv`                → env-variable names (awk splits on `=`)
  # See dump.tcsh for details.
  run tcsh "$TESTS_DIR/deactivate-state/dump.tcsh" "$pre" "$post"
  assert_success

  _assert_state_restored tcsh "$pre" "$post"
}
