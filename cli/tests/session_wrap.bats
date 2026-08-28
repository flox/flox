#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end tests for `[plugin-hooks].session-wrap`: the full path from a
# manifest declaration through hook discovery in the rendered environment to
# exec'ing the plugin's wrapper, which re-enters the activation via the
# ctx's `inner_argv` with the `_FLOX_SESSION_WRAPPED` marker set.
#
# The fixture plugin is a package built on the fly with `nix store add`,
# whose hook logs its invocation, snapshots its ctx for assertions, and
# re-execs the inner activation — the host-boundary consumption style.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=session-wrap

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
  export FLOX_FEATURES_PLUGIN_HOOKS=true
  export WRAP_LOG="${BATS_TEST_TMPDIR?}/wrapper-log"
}

teardown() {
  unset FLOX_FEATURES_PLUGIN_HOOKS
  if [ -n "${PROJECT_DIR:-}" ]; then
    wait_for_activations "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Build and install a minimal wrapper plugin package into the current
# project. Its session-wrap hook records that it ran, snapshots its ctx,
# exports the re-entry marker, and re-execs the host-side inner argv.
setup_wrapper_plugin() {
  mkdir -p "$BATS_TEST_TMPDIR/test-wrapper/etc/flox/hooks/session-wrap.d"
  cat > "$BATS_TEST_TMPDIR/test-wrapper/etc/flox/hooks/session-wrap.d/test-wrapper" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
echo "wrapper-ran plugin=${FLOX_PLUGIN_NAME}" >> "$WRAP_LOG"
cp "$FLOX_HOOK_CTX" "$WRAP_LOG.ctx"
_FLOX_SESSION_WRAPPED="$("$FLOX_HOOK_JQ" -r '.wrap_scope' "$FLOX_HOOK_CTX")"
export _FLOX_SESSION_WRAPPED
mapfile -t inner_argv < <("$FLOX_HOOK_JQ" -r '.inner_argv[]' "$FLOX_HOOK_CTX")
exec "${inner_argv[@]}"
EOF
  chmod +x "$BATS_TEST_TMPDIR/test-wrapper/etc/flox/hooks/session-wrap.d/test-wrapper"
  pkg_store_path="$(nix --extra-experimental-features nix-command \
    store add --name test-wrapper "$BATS_TEST_TMPDIR/test-wrapper")"
  run "$FLOX_BIN" install "$pkg_store_path"
  assert_success
}

declare_wrapper() {
  {
    cat "$PROJECT_DIR/.flox/env/manifest.toml"
    echo
    echo '[plugin-hooks]'
    echo 'session-wrap = "test-wrapper"'
  } | "$FLOX_BIN" edit -f -
}

# ---------------------------------------------------------------------------- #

# bats test_tags=session-wrap:wraps
@test "session-wrap: declared hook wraps the activation and re-enters via inner_argv" {
  project_setup
  setup_wrapper_plugin
  declare_wrapper

  run "$FLOX_BIN" activate -- true
  assert_success

  # The wrapper ran exactly once: the marker made the re-entered activation
  # skip dispatch instead of recursing.
  run cat "$WRAP_LOG"
  assert_success
  assert_output "wrapper-ran plugin=test-wrapper"
}

# bats test_tags=session-wrap:ctx
@test "session-wrap: the ctx carries the invocation payload and scope" {
  project_setup
  setup_wrapper_plugin
  declare_wrapper

  run "$FLOX_BIN" activate -- echo hook-ctx-probe
  assert_success
  assert_output --partial "hook-ctx-probe"

  run jq -r '.ctx_version' "$WRAP_LOG.ctx"
  assert_success
  assert_output "1"

  # The full exec-command payload is serialized so container-style wrappers
  # can compose their own in-boundary command.
  run jq -r '.invocation_type | to_entries[0].value | join(" ")' "$WRAP_LOG.ctx"
  assert_success
  assert_output "echo hook-ctx-probe"

  run jq -r '.wrap_scope | length' "$WRAP_LOG.ctx"
  assert_success
  refute_output "0"

  run jq -r '.plugin_table' "$WRAP_LOG.ctx"
  assert_success
  assert_output "null"
}

# bats test_tags=session-wrap:feature-off
@test "session-wrap: without the feature flag the declaration warns and is ignored" {
  project_setup
  setup_wrapper_plugin
  declare_wrapper

  unset FLOX_FEATURES_PLUGIN_HOOKS
  run "$FLOX_BIN" activate -- true
  assert_success
  assert_output --partial "Ignored [plugin-hooks]"
  assert [ ! -e "$WRAP_LOG" ]
}

# bats test_tags=session-wrap:undeclared
@test "session-wrap: a shipped but undeclared hook is ignored with a warning" {
  project_setup
  setup_wrapper_plugin

  run "$FLOX_BIN" activate -- true
  assert_success
  assert_output --partial "Ignored session-wrap hook 'test-wrapper'"
  assert [ ! -e "$WRAP_LOG" ]
}

# bats test_tags=session-wrap:in-place
@test "session-wrap: in-place activation of a wrapping environment errors" {
  project_setup
  setup_wrapper_plugin
  declare_wrapper

  # stdout is not a tty under bats, so a bare `flox activate` plans an
  # in-place activation.
  run "$FLOX_BIN" activate
  assert_failure
  assert_output --partial "Cannot activate in-place"
  assert [ ! -e "$WRAP_LOG" ]
}

# bats test_tags=session-wrap:nested
@test "session-wrap: activating inside a foreign wrap boundary errors" {
  project_setup
  setup_wrapper_plugin
  declare_wrapper

  _FLOX_SESSION_WRAPPED="some-other-scope" run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "inside another environment's session-wrap boundary"
  assert [ ! -e "$WRAP_LOG" ]
}

# bats test_tags=session-wrap:missing-hook
@test "session-wrap: declaring a plugin that ships no hook errors" {
  project_setup
  setup_wrapper_plugin

  {
    cat "$PROJECT_DIR/.flox/env/manifest.toml"
    echo
    echo '[plugin-hooks]'
    echo 'session-wrap = "no-such-plugin"'
  } | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "declares a session-wrap hook but the environment provides none"
}
