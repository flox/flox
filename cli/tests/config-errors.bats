#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test that a config that does not parse creates no telemetry state
# (no device ID, no Sentry client), and that neither a config that does not
# parse nor an empty or relative cache, data or state dir puts files into
# the current directory.
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
  assert_success

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
  # Show user messages so that the first run prints the telemetry notice.
  # A first run that hides the notice creates no device ID and starts no
  # Sentry client.
  export RUST_LOG="flox::utils::message=info,flox_core::sentry=debug"

  # A valid config with metrics on initializes Sentry.
  run "$FLOX_BIN" --help
  assert_success
  assert_output --partial "Initializing Sentry"

  FLOX_DISABLE_METRICS=1 run "$FLOX_BIN" --help
  assert_success
  refute_output --partial "Initializing Sentry"
}

@test "empty FLOX_DATA_DIR and FLOX_CACHE_DIR use the default directories" {
  export XDG_DATA_HOME="$BATS_TEST_TMPDIR/xdg-data"
  export XDG_CACHE_HOME="$BATS_TEST_TMPDIR/xdg-cache"

  FLOX_DATA_DIR="" FLOX_CACHE_DIR="" run "$FLOX_BIN" --help
  assert_success
  FLOX_DATA_DIR="" FLOX_CACHE_DIR="" run "$FLOX_BIN" envs --active
  assert_success

  assert_cwd_empty
  assert [ -f "$XDG_DATA_HOME/flox/metrics-uuid" ]
  assert [ -f "$XDG_CACHE_HOME/flox/metrics-lock" ]
}

@test "relative FLOX_DATA_DIR and FLOX_CACHE_DIR are rejected" {
  FLOX_DATA_DIR=. FLOX_CACHE_DIR=. run "$FLOX_BIN" --help
  assert_success

  FLOX_DATA_DIR=. FLOX_CACHE_DIR=. run "$FLOX_BIN" envs --active
  assert_failure
  assert_output --partial "cache_dir '.' is not an absolute path"

  assert_cwd_empty
}

@test "flox config --set rejects a relative dir" {
  run "$FLOX_BIN" config --set cache_dir relative-cache-dir
  assert_failure
  assert_output --partial "cache_dir 'relative-cache-dir' is not an absolute path"

  # The config is unchanged, so commands still run.
  run "$FLOX_BIN" config
  assert_success
  refute_output --partial "relative-cache-dir"

  assert_cwd_empty
}
