#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end state-restoration test for `flox deactivate --print-script`.
#
# Snapshots the full shell state — functions/aliases, shell options, and
# all variable *names* (including non-exported locals) — before activation
# and after deactivation, then diffs the two. A clean diff means
# activate/deactivate is fully reversible: nothing leaks past the
# deactivate boundary.
#
# Covers bash, zsh, fish, and tcsh. The dump primitives are shell-specific
# (bash `compgen`/`declare -f`/`shopt`, zsh `${(ko)functions}`/`setopt`,
# fish `functions --names`/`set --names`, tcsh `alias`/`set`/`setenv`), so
# each shell has its own helper under shell-state-restoration/dump.<shell>.
#
# The snapshot writes a normalized, sorted view (LC_ALL=C) to a file. The
# test diffs pre vs. post, filtering shell-internal noise (LINENO, RANDOM,
# BASH_COMMAND, history vars, etc.) and the test harness's own setup vars.
#
# Division of labor with deactivate.bats:
#   - deactivate.bats owns *exported environment* restoration: it diffs
#     `env` *values* (not just names) before/after, across the in-place,
#     subshell, and interactive (pty) invocation modes, and also checks
#     prompt and individual user-set variables.
#   - this file owns *full shell-state* restoration by *name* (functions,
#     aliases, options, non-exported vars) — the surface `env` cannot see
#     — for the in-place invocation mode only.
# Neither subsumes the other; keep both.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=deactivate,shell-state-restoration

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

# Names that come from the test harness or the host OS, not from
# activate/deactivate. Filtered out of both snapshots (by the dump.<shell>
# helpers) so the diff only surfaces state that flox itself touched. These
# are noise, not leaks, so — unlike the allow-list below — they are never
# leak-checked.
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
# deactivate.bats keeps a parallel env-diff noise list; the names common to
# both (NIX_SSL_CERT_FILE, PATH_LOCALE) must stay in sync.
export _TEST_HARNESS_NOISE_RE='^(__FT_RAN_.*|_FLOX_LOCAL_DEV|_FLOX_TEST_SUITE_MODE|_FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS|_FLOX_USE_CATALOG_MOCK|__CF_USER_TEXT_ENCODING|__NIX_DARWIN_SET_ENVIRONMENT_DONE|PATH_LOCALE|NIX_SSL_CERT_FILE)$'

# ---------------------------------------------------------------------------- #
# Intentional ("allow-listed") leaks.
#
# These names are *expected* to differ between the pre-activate and
# post-deactivate snapshots. Unlike the noise filter above, they are NOT
# stripped from the dumps — the dumps emit every leaked name and
# `_assert_state_restored` classifies each one. This keeps the allow-list
# honest: a leak that gets fixed upstream stops appearing, and the test
# then FAILS ("no longer leaks") until its entry is removed, instead of the
# allow-list silently rotting. Add an entry only for a genuinely intentional
# leak, with a justification.
#
# Seen in every shell:
#   _FLOX_SOURCED_PROFILE_SCRIPTS — tracking var, intentionally NOT updated
#                                   on deactivate (see deactivate.bats).
_EXPECTED_LEAKS_SHARED='_FLOX_SOURCED_PROFILE_SCRIPTS'

# bash-only:
#   _flox_hook     — auto-activate prompt hook, left registered so re-entry
#                    on `cd` still fires (DEV-86).
#   PROMPT_COMMAND — carries _flox_hook into the user's shell.
_EXPECTED_LEAKS_BASH='_flox_hook PROMPT_COMMAND'

# zsh-only:
#   _flox_hook     — as above.
#   _comp_assocs / chpwd_functions / precmd_functions
#                  — compinit / zsh hook plumbing pulled in by
#                    activate.d/zsh's compinit call.
#   nohashdirs     — compinit toggles nohashdirs; deactivate over-restores
#                    to the zsh default (`setopt hashdirs`) rather than the
#                    pre-activate state.
_EXPECTED_LEAKS_ZSH='_flox_hook _comp_assocs chpwd_functions precmd_functions nohashdirs'

# fish: no intentional leaks beyond the shared set.
_EXPECTED_LEAKS_FISH=''

# tcsh (csh has aliases, not functions):
#   precmd / cwdcmd       — activate.tcsh registers these aliases to re-run
#                           `flox hook-env`; not removed by deactivate.
#   _already_sourced_args — activate-local helper var, not cleaned up.
_EXPECTED_LEAKS_TCSH='_already_sourced_args cwdcmd precmd'

# Conditional (environment-dependent) leaks: allowed when present, but NOT
# stale-checked. Their absence reflects the environment rather than a fix, so
# requiring them would make the test pass or fail depending on where it runs.
# Keep this minimal — prefer _EXPECTED_LEAKS for anything deterministic.
#
# zsh:
#   MANPATH / manpath — when MANPATH is set pre-activate and zsh's `manpath`
#                       is tied to it (CI runners and most interactive shells
#                       do this; a bare bats process does not), zsh deactivate
#                       fails to restore them: post has neither. Tracked in
#                       DEV-86.
_CONDITIONAL_LEAKS_ZSH='MANPATH manpath'

# Echo the deterministic allow-listed leak names for $1 (shared + per-shell).
_expected_leaks_for() {
  local shell="$1"
  printf '%s' "$_EXPECTED_LEAKS_SHARED"
  case "$shell" in
    bash) printf ' %s' "$_EXPECTED_LEAKS_BASH" ;;
    zsh) printf ' %s' "$_EXPECTED_LEAKS_ZSH" ;;
    fish) printf ' %s' "$_EXPECTED_LEAKS_FISH" ;;
    tcsh) printf ' %s' "$_EXPECTED_LEAKS_TCSH" ;;
  esac
}

# Echo the conditional (environment-dependent) leak names for $1.
_conditional_leaks_for() {
  case "$1" in
    zsh) printf '%s' "$_CONDITIONAL_LEAKS_ZSH" ;;
  esac
}

# Compare the pre-activate and post-deactivate snapshots for $shell.
#
# The dumps emit every leaked name (only true shell-internal volatiles and
# host/harness noise are pre-filtered), so the raw diff is the full set of
# names activate/deactivate failed to restore. Each differing name is
# classified against this shell's allow-lists:
#   - in _EXPECTED_LEAKS (deterministic)    → reported, OK
#   - in _CONDITIONAL_LEAKS (env-dependent) → reported, OK
#   - in neither                            → unexpected leak, FAIL
# Stale check: an _EXPECTED_LEAKS name not observed leaking FAILs the test
# (it was fixed upstream — remove it; no silent rot). _CONDITIONAL_LEAKS are
# exempt, since their absence reflects the environment rather than a fix.
_assert_state_restored() {
  local shell="$1" pre="$2" post="$3"
  local expected conditional
  expected="$(_expected_leaks_for "$shell")"
  conditional="$(_conditional_leaks_for "$shell")"

  # Bare names that differ between the snapshots (drop the section headers
  # and diff's own markers; what remains is one name per line).
  local observed
  observed="$(
    diff "$pre" "$post" \
      | grep -E '^[<>]' \
      | sed -E 's/^[<>] ?//' \
      | grep -vE '^=== .* ===$' \
      | grep -vE '^[[:space:]]*$' \
      | sort -u
  )"

  local present="" unexpected="" missing="" name e ok
  while IFS= read -r name; do
    [ -z "$name" ] && continue
    ok=0
    for e in $expected $conditional; do
      [ "$e" = "$name" ] && ok=1 && break
    done
    if [ "$ok" -eq 1 ]; then
      present+="    $name"$'\n'
    else
      unexpected+="    $name"$'\n'
    fi
  done <<< "$observed"

  # Stale check covers only the deterministic _EXPECTED_LEAKS.
  for e in $expected; do
    grep -qxF "$e" <<< "$observed" || missing+="    $e"$'\n'
  done

  # Always surface the leaks we saw, even when all are allow-listed, so
  # tightening opportunities stay visible (bats shows fd 3 on pass too).
  {
    echo "[$shell] allow-listed leaks observed:"
    [ -n "$present" ] && printf '%s' "$present" || echo "    (none)"
  } >&3

  if [ -n "$unexpected" ] || [ -n "$missing" ]; then
    {
      echo "Shell state not restored as expected after deactivate ($shell)."
      if [ -n "$unexpected" ]; then
        echo
        echo "Unexpected leaks (post differs from pre, not allow-listed)."
        echo "Fix the leak, or if it is intentional add the name to the"
        echo "_EXPECTED_LEAKS list for $shell with a justification:"
        printf '%s' "$unexpected"
      fi
      if [ -n "$missing" ]; then
        echo
        echo "Allow-listed leaks that no longer occur (fixed upstream)."
        echo "Remove these from the _EXPECTED_LEAKS list for $shell:"
        printf '%s' "$missing"
      fi
    } >&2
    fail "$shell shell state not restored as expected (see report above)"
  fi
}

# bats test_tags=deactivate,shell-state-restoration:bash
@test "bash: deactivate restores pre-activation shell state" {
  project_setup

  pre="${BATS_TEST_TMPDIR}/pre.txt"
  post="${BATS_TEST_TMPDIR}/post.txt"

  # bash dump primitives:
  #   - `set -o` / `shopt -p`   → shell options
  #   - `compgen -A function` / `declare -f`
  #                             → function names (sorted) and bodies
  #   - `compgen -v`            → variable names, including non-exported
  # The actual filter/dump lives in dump.bash; see comments there.
  run bash "$TESTS_DIR/shell-state-restoration/dump.bash" "$pre" "$post"
  assert_success

  _assert_state_restored bash "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,shell-state-restoration:zsh
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
  run zsh "$TESTS_DIR/shell-state-restoration/dump.zsh" "$pre" "$post"
  assert_success

  _assert_state_restored zsh "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,shell-state-restoration:fish
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
  run fish "$TESTS_DIR/shell-state-restoration/dump.fish" "$pre" "$post"
  assert_success

  _assert_state_restored fish "$pre" "$post"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate,shell-state-restoration:tcsh
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
  run tcsh "$TESTS_DIR/shell-state-restoration/dump.tcsh" "$pre" "$post"
  assert_success

  _assert_state_restored tcsh "$pre" "$post"
}
