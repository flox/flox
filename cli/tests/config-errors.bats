#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that a config that does not parse is an error for every invocation,
# and creates no telemetry state (no device ID, no Sentry client) and no files
# in the current directory.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=config,telemetry

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  # Turn metrics on, as on a user's machine, so that falling back to the
  # default config would create telemetry state.
  export FLOX_DISABLE_METRICS=false
  export _FLOX_METRICS_URL_OVERRIDE="http://127.0.0.1:1/legacy"
  export _FLOX_METRICS_URL_V2_OVERRIDE="http://127.0.0.1:1/v2"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/cwd"
  mkdir -p "$PROJECT_DIR"
  cd "$PROJECT_DIR" || return
}

assert_cwd_empty() {
  run ls -A "$PROJECT_DIR"
  assert_success
  assert_output ""
}

# ---------------------------------------------------------------------------- #

@test "invalid FLOX_DISABLE_METRICS creates no device ID" {
  FLOX_DISABLE_METRICS=1 run "$FLOX_BIN" --help
  assert_failure
  assert_output --partial "Could not parse config"

  # Allow the detached telemetry sender, which would log into the current
  # directory if it ran with the default config.
  _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=false _FLOX_FORCE_FLUSH_METRICS=true \
    FLOX_DISABLE_METRICS=garbage run "$FLOX_BIN" envs --active
  assert_failure
  assert_output --partial "Could not parse config"

  assert_cwd_empty
  assert [ ! -e "$FLOX_DATA_DIR/metrics-uuid" ]
}

@test "malformed flox.toml creates no device ID" {
  # A quoted boolean is a string, which does not parse as `disable_metrics`.
  unset FLOX_DISABLE_METRICS
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'disable_metrics = "true"' > "$FLOX_CONFIG_DIR/flox.toml"

  run "$FLOX_BIN" envs --active
  assert_failure
  assert_output --partial "Could not parse config"

  assert_cwd_empty
  assert [ ! -e "$FLOX_DATA_DIR/metrics-uuid" ]
}

@test "invalid config does not initialize Sentry" {
  export FLOX_SENTRY_DSN="http://public@127.0.0.1:1/1"
  export RUST_LOG="flox_core::sentry=debug"

  # A valid config with metrics on initializes Sentry.
  run "$FLOX_BIN" --help
  assert_success
  assert_output --partial "Initializing Sentry"

  FLOX_DISABLE_METRICS=1 run "$FLOX_BIN" --help
  assert_failure
  refute_output --partial "Initializing Sentry"
}
