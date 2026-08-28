#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end tests for `[plugin-hooks].env` and `[plugin-hooks].sidecar`:
# CLI-side resolution recorded into the activation ctx, env-hook dispatch
# at start/attach through the double-set channel (fail-closed), and
# sidecar supervision by the executive (spawned before readiness, reaped
# at teardown).
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=plugin-hooks-exec

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
  export HOOK_LOG="${BATS_TEST_TMPDIR?}/hook-log"
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

# Build and install a plugin package shipping an env hook that logs its
# invocation and contributes variables derived from its ctx.
setup_env_hook_plugin() {
  script="${1:?}"
  mkdir -p "$BATS_TEST_TMPDIR/test-env-plugin/etc/flox/hooks/env.d"
  printf '%s\n' "$script" \
    > "$BATS_TEST_TMPDIR/test-env-plugin/etc/flox/hooks/env.d/test-env-plugin"
  chmod +x "$BATS_TEST_TMPDIR/test-env-plugin/etc/flox/hooks/env.d/test-env-plugin"
  pkg_store_path="$(nix --extra-experimental-features nix-command \
    store add --name test-env-plugin "$BATS_TEST_TMPDIR/test-env-plugin")"
  run "$FLOX_BIN" install "$pkg_store_path"
  assert_success
}

declare_env_hook() {
  {
    cat "$PROJECT_DIR/.flox/env/manifest.toml"
    echo
    echo '[plugin-hooks]'
    echo 'env = ["test-env-plugin"]'
  } | "$FLOX_BIN" edit -f -
}

# Sidecar fixture: writes its pid to a file, then sleeps until terminated.
setup_sidecar_plugin() {
  mkdir -p "$BATS_TEST_TMPDIR/test-sidecar/etc/flox/hooks/sidecar.d"
  cat > "$BATS_TEST_TMPDIR/test-sidecar/etc/flox/hooks/sidecar.d/test-sidecar" <<EOF
#!/usr/bin/env bash
set -euo pipefail
echo "\$\$" > "$HOOK_LOG.pid"
"\$FLOX_HOOK_JQ" -r '.runtime_dir' "\$FLOX_HOOK_CTX" > "$HOOK_LOG.runtime_dir"
trap 'exit 0' TERM
while :; do sleep 1; done
EOF
  chmod +x "$BATS_TEST_TMPDIR/test-sidecar/etc/flox/hooks/sidecar.d/test-sidecar"
  pkg_store_path="$(nix --extra-experimental-features nix-command \
    store add --name test-sidecar "$BATS_TEST_TMPDIR/test-sidecar")"
  run "$FLOX_BIN" install "$pkg_store_path"
  assert_success
}

declare_sidecar() {
  {
    cat "$PROJECT_DIR/.flox/env/manifest.toml"
    echo
    echo '[plugin-hooks]'
    echo 'sidecar = ["test-sidecar"]'
  } | "$FLOX_BIN" edit -f -
}

# ---------------------------------------------------------------------------- #

# bats test_tags=plugin-hooks-exec:env-contributes
@test "env hook: contributed variables reach the activation" {
  project_setup
  setup_env_hook_plugin '#!/usr/bin/env bash
phase="$("$FLOX_HOOK_JQ" -r .phase "$FLOX_HOOK_CTX")"
echo "env-hook-ran phase=$phase" >> "$HOOK_LOG"
printf "{\"INJECTED_BY_PLUGIN\": \"hello-$phase\"}"'
  declare_env_hook

  run "$FLOX_BIN" activate -- bash -c 'echo "value=$INJECTED_BY_PLUGIN"'
  assert_success
  assert_output --partial "value=hello-attach"

  # The hook ran for both dispatch points: activation start and the attach.
  run cat "$HOOK_LOG"
  assert_success
  assert_line "env-hook-ran phase=start"
  assert_line "env-hook-ran phase=attach"
}

# bats test_tags=plugin-hooks-exec:env-reserved
@test "env hook: a _FLOX_-prefixed key fails the activation" {
  project_setup
  setup_env_hook_plugin '#!/usr/bin/env bash
printf "{\"_FLOX_SESSION_WRAPPED\": \"forged\"}"'
  declare_env_hook

  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "reserved"
}

# bats test_tags=plugin-hooks-exec:env-fail-closed
@test "env hook: a failing hook fails the activation" {
  project_setup
  setup_env_hook_plugin '#!/usr/bin/env bash
echo "hook diagnostics" >&2
exit 3'
  declare_env_hook

  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "failed with"
}

# bats test_tags=plugin-hooks-exec:env-missing
@test "env hook: declaring a plugin that ships no hook errors" {
  project_setup

  {
    cat "$PROJECT_DIR/.flox/env/manifest.toml"
    echo
    echo '[plugin-hooks]'
    echo 'env = ["no-such-plugin"]'
  } | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" activate -- true
  assert_failure
  assert_output --partial "declares a env hook but the environment provides none"
}

# bats test_tags=plugin-hooks-exec:sidecar-lifecycle
@test "sidecar: alive for the activation, reaped at teardown" {
  project_setup
  setup_sidecar_plugin
  declare_sidecar

  run "$FLOX_BIN" activate -- bash -c '
    for _ in $(seq 50); do
      [ -f "$HOOK_LOG.pid" ] && break
      sleep 0.2
    done
    pid="$(cat "$HOOK_LOG.pid")"
    kill -0 "$pid" && echo "sidecar-alive pid=$pid"'
  assert_success
  assert_output --partial "sidecar-alive"

  # After the last shell exits, the executive tears down: sidecar dies and
  # its private runtime dir is removed.
  wait_for_activations "$PROJECT_DIR"
  pid="$(cat "$HOOK_LOG.pid")"
  for _ in $(seq 50); do
    kill -0 "$pid" 2>/dev/null || break
    sleep 0.2
  done
  run kill -0 "$pid"
  assert_failure
  runtime_dir="$(cat "$HOOK_LOG.runtime_dir")"
  assert [ ! -e "$runtime_dir" ]
}

# bats test_tags=plugin-hooks-exec:feature-off
@test "env hook: without the feature flag the declaration warns and is ignored" {
  project_setup
  setup_env_hook_plugin '#!/usr/bin/env bash
echo ran >> "$HOOK_LOG"
printf "{}"'
  declare_env_hook

  unset FLOX_FEATURES_PLUGIN_HOOKS
  run "$FLOX_BIN" activate -- true
  assert_success
  assert_output --partial "Ignored [plugin-hooks]"
  assert [ ! -e "$HOOK_LOG" ]
}
