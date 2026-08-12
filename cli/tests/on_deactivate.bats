#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end tests for `[hook.on-deactivate]`: the full path from manifest
# entry to buildenv emission to the executive running the hook when the last
# attachment detaches from a start.
#
# The hook runs asynchronously in the executive, so tests synchronize on
# `wait_for_activations` (the activation state dir is only removed after the
# hook has run) before asserting on the hook's side effects.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=on-deactivate

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
  # `deactivate --print-script` after an in-place activation needs the
  # `_FLOX_PROMPT_HOOK_VERSION` marker, so opt out of the suite-wide
  # `disable_hook = true`.
  enable_prompt_hook
  home_setup test
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
  export MARKER="${BATS_TEST_TMPDIR?}/on-deactivate-marker"
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

# Write a manifest whose on-activate exports FOO and whose on-deactivate
# appends its value to $MARKER, proving both that the hook ran (exactly as
# many times as there are lines) and that it sees the environment as
# hook.on-activate left it.
_write_on_deactivate_manifest() {
  cat <<EOF | "$FLOX_BIN" edit -f -
schema-version = "1.15.0"

[options]

[hook]
on-activate = 'export FOO="from-activate"'
on-deactivate = 'echo "\$FOO" >> "$MARKER"'
EOF
}

# bats test_tags=on-deactivate:deactivate
@test "hook.on-deactivate runs on explicit deactivate with on-activate's environment" {
  project_setup
  _write_on_deactivate_manifest
  FLOX_SHELL="bash" run --separate-stderr bash -c '
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
  '
  assert_success

  wait_for_activations "$PROJECT_DIR"
  run cat "$MARKER"
  assert_success
  assert_output "from-activate"
}

# bats test_tags=on-deactivate:command-exit
@test "hook.on-deactivate runs when a command-mode activation exits" {
  project_setup
  _write_on_deactivate_manifest
  run "$FLOX_BIN" activate -- true
  assert_success

  wait_for_activations "$PROJECT_DIR"
  run cat "$MARKER"
  assert_success
  assert_output "from-activate"
}

# bats test_tags=on-deactivate:attach
@test "hook.on-deactivate does not run while other attachments remain" {
  project_setup
  _write_on_deactivate_manifest
  FLOX_SHELL="bash" run --separate-stderr bash -c '
    eval "$($FLOX_BIN activate --print-script)"
    # A second attachment to the same start comes and goes; the hook must
    # not fire because this shell is still attached.
    "$FLOX_BIN" activate -- true
    # Firing is asynchronous, so give the executive a moment to (wrongly)
    # act before asserting the marker is absent.
    sleep 1
    if [ -e "$MARKER" ]; then
      echo "marker-too-early"
    else
      echo "no-marker-while-attached"
    fi
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
  '
  assert_success
  assert_line "no-marker-while-attached"
  refute_line "marker-too-early"

  wait_for_activations "$PROJECT_DIR"
  # The hook ran exactly once for the start.
  run cat "$MARKER"
  assert_success
  assert_output "from-activate"
}

# bats test_tags=on-deactivate:failure
@test "a failing hook.on-deactivate does not block deactivation or cleanup" {
  project_setup
  cat <<EOF | "$FLOX_BIN" edit -f -
schema-version = "1.15.0"

[options]

[hook]
on-deactivate = "exit 1"
EOF
  run "$FLOX_BIN" activate -- true
  assert_success

  # Cleanup completes (state dir removed) despite the failing hook;
  # wait_for_activations fails the test if it doesn't.
  wait_for_activations "$PROJECT_DIR"
}
