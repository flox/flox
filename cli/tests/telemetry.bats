#! /usr/bin/env bats
# -*- mode: bats; -*-

# Test telemetry scheduling on the shared command-exit path. `envs --active`
# records telemetry without needing an environment or network access.
load test_support.bash

# bats file_tags=telemetry

setup() {
  common_test_setup
  setup_isolated_flox
  cd "$BATS_TEST_TMPDIR"
  export FLOX_DISABLE_METRICS=false
  unset _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS _FLOX_FORCE_FLUSH_METRICS
  export _FLOX_METRICS_URL_V2_OVERRIDE="http://127.0.0.1:1/v2"
}

wait_for_telemetry_flush() {
  local log_file="$FLOX_CACHE_DIR/log/send-telemetry.log"
  for _ in $(seq 1 100); do
    if grep -qs "send-telemetry flush complete" "$log_file"; then
      return 0
    fi
    sleep 0.1
  done
  echo "Timed out waiting for telemetry flush" >&2
  cat "$log_file" >&2
  return 1
}

teardown() {
  # The parent creates the log before spawning. Wait for buffer writes to finish
  # before deleting directories that the detached sender may still recreate.
  if [[ -f "$FLOX_CACHE_DIR/log/send-telemetry.log" ]]; then
    wait_for_telemetry_flush || return 1
  fi
  common_test_teardown
}

@test "command exit skips fresh telemetry unless flushing is forced" {
  run "$FLOX_BIN" envs --active
  assert_success
  run "$FLOX_BIN" envs --active
  assert_success
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]
  # Only older releases write the legacy buffer.
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  # The parent opens this file before spawning, so no wait is needed.
  refute [ -e "$FLOX_CACHE_DIR/log/send-telemetry.log" ]

  export _FLOX_FORCE_FLUSH_METRICS=true
  run "$FLOX_BIN" envs --active
  assert_success
  assert [ -f "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
}

@test "command exit spawns telemetry when only v2 events expire" {
  run "$FLOX_BIN" envs --active
  assert_success
  jq -c '.event_timestamp = 0' "$FLOX_DATA_DIR/events-v2.json" > "$BATS_TEST_TMPDIR/expired.json"
  mv "$BATS_TEST_TMPDIR/expired.json" "$FLOX_DATA_DIR/events-v2.json"

  run "$FLOX_BIN" envs --active
  assert_success
  assert [ -f "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
}

# Older releases buffer the legacy stream here, and only they send it. Every
# subcommand and bare `flox` delete it, opted out or not. The device ID and the
# lock stay.
@test "legacy metrics buffer is deleted even with telemetry disabled" {
  export FLOX_DISABLE_METRICS=true
  local buffer="$FLOX_CACHE_DIR/metrics-events-v2.json"
  local device_id="7b3c9a52-1f0e-4d2b-9c6a-3e5f8d1a2b4c"
  mkdir -p "$FLOX_CACHE_DIR" "$FLOX_DATA_DIR"
  printf '%s' "$device_id" > "$FLOX_DATA_DIR/metrics-uuid"
  : > "$FLOX_CACHE_DIR/metrics-lock"

  echo '{"subcommand":"search","search_term":"hello"}' > "$buffer"
  run "$FLOX_BIN" envs --active
  assert_success
  refute [ -e "$buffer" ]

  echo '{"subcommand":"search","search_term":"hello"}' > "$buffer"
  run "$FLOX_BIN"
  assert_success
  refute [ -e "$buffer" ]

  run cat "$FLOX_DATA_DIR/metrics-uuid"
  assert_output "$device_id"
  assert [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
}

# An unparseable device ID disables telemetry; commands still succeed.
@test "unparseable device ID warns and records nothing" {
  mkdir -p "$FLOX_DATA_DIR"
  : > "$FLOX_DATA_DIR/metrics-uuid"

  run "$FLOX_BIN" envs --active
  assert_success
  assert_output --partial "Could not parse the metrics UUID of this installation in"
  assert_output --partial "flox-data/metrics-uuid"
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]

  # `reset-metrics` replaces the unparseable ID.
  run "$FLOX_BIN" reset-metrics
  assert_success
  run "$FLOX_BIN" envs --active
  assert_success
  run cat "$FLOX_DATA_DIR/metrics-uuid"
  assert_output --regexp '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
}

# Force delivery to a black-hole endpoint to exercise detached submission even
# with fresh events. A refused connection would not expose a blocking sender.
enable_blocking_telemetry_endpoints() {
  export _FLOX_METRICS_URL_V2_OVERRIDE="https://192.0.2.1/v2"
  export _FLOX_FORCE_FLUSH_METRICS=true
}

@test "command exit does not wait for telemetry delivery" {
  enable_blocking_telemetry_endpoints
  run timeout 1 "$FLOX_BIN" envs --active
  assert_success
}

# Unit tests cover successful delivery. This checks that the detached child
# inherits the endpoint override and retains events after failed delivery.
@test "detached telemetry submission preserves events when delivery fails" {
  enable_blocking_telemetry_endpoints
  run "$FLOX_BIN" envs --active
  assert_success

  local log_file="$FLOX_CACHE_DIR/log/send-telemetry.log"
  wait_for_telemetry_flush
  run grep "Sending v2 events" "$log_file"
  assert_output --partial "192.0.2.1"
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]
}
