#! /usr/bin/env bats
# -*- mode: bats; -*-

# Test telemetry scheduling on the shared command-exit path. `envs --active`
# records both pipelines without needing an environment or network access.
load test_support.bash

# bats file_tags=telemetry

setup() {
  common_test_setup
  setup_isolated_flox
  cd "$BATS_TEST_TMPDIR"
  export FLOX_DISABLE_METRICS=false
  unset _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS _FLOX_FORCE_FLUSH_METRICS
  export _FLOX_METRICS_URL_OVERRIDE="http://127.0.0.1:1/legacy"
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
  assert [ -s "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
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

@test "command exit spawns telemetry when only legacy metrics expire" {
  run "$FLOX_BIN" envs --active
  assert_success
  jq -c '.timestamp = [2020, 1, 0, 0, 0, 0, 0, 0, 0]' "$FLOX_CACHE_DIR/metrics-events-v2.json" > "$BATS_TEST_TMPDIR/expired.json"
  mv "$BATS_TEST_TMPDIR/expired.json" "$FLOX_CACHE_DIR/metrics-events-v2.json"

  run "$FLOX_BIN" envs --active
  assert_success
  assert [ -f "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
}

# Force delivery to a black-hole endpoint to exercise detached submission even
# with fresh events. A refused connection would not expose a blocking sender.
enable_blocking_telemetry_endpoints() {
  export _FLOX_METRICS_URL_OVERRIDE="https://192.0.2.1/legacy"
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

# The device ID is created together with the one-time telemetry notice.
# A first run that cannot show the notice creates no telemetry files and
# records nothing.
# "disable_metrics" appears in the notice, in its list of opt-out commands.
refute_telemetry_files() {
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
  refute [ -e "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
}

@test "quiet first run creates no device ID until the notice is shown" {
  unset RUST_LOG

  # Forcing a flush spawns no sender either.
  run env _FLOX_FORCE_FLUSH_METRICS=true "$FLOX_BIN" -q envs --active
  assert_success
  refute_output --partial "disable_metrics"
  refute_telemetry_files

  run "$FLOX_BIN" envs --active
  assert_success
  assert_output --partial "disable_metrics"
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]

  # After the notice, quiet runs record with the same device ID.
  local uuid events
  uuid="$(cat "$FLOX_DATA_DIR/metrics-uuid")"
  events="$(wc -l < "$FLOX_DATA_DIR/events-v2.json")"
  run "$FLOX_BIN" -q envs --active
  assert_success
  refute_output --partial "disable_metrics"
  assert_equal "$(cat "$FLOX_DATA_DIR/metrics-uuid")" "$uuid"
  assert [ "$(wc -l < "$FLOX_DATA_DIR/events-v2.json")" -gt "$events" ]
}

@test "first run with RUST_LOG hiding messages creates no device ID" {
  run env RUST_LOG=error "$FLOX_BIN" envs --active
  assert_success
  refute_output --partial "disable_metrics"
  refute_telemetry_files

  # A RUST_LOG filter that keeps user-facing messages shows the notice.
  run env RUST_LOG=info "$FLOX_BIN" envs --active
  assert_success
  assert_output --partial "disable_metrics"
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
}

@test "shell completion creates no device ID and prints no notice" {
  unset RUST_LOG

  run "$FLOX_BIN" --bpaf-complete-rev=8 en
  assert_success
  refute_output --partial "disable_metrics"
  refute_telemetry_files
}

@test "detached telemetry sender creates no device ID" {
  unset RUST_LOG

  run "$FLOX_BIN" send-telemetry
  assert_success
  refute_output --partial "disable_metrics"
  refute_telemetry_files
}

# Unlike opting out, deferring is not passed on to child processes,
# so the next run that can show the notice creates the device ID.
@test "quiet first run does not turn telemetry off for child processes" {
  unset RUST_LOG
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true

  "$FLOX_BIN" -q init
  run --separate-stderr "$FLOX_BIN" -q activate -c 'printenv FLOX_DISABLE_METRICS'
  assert_success
  assert_output "false"
  refute_telemetry_files

  FLOX_DISABLE_METRICS=true wait_for_activations "$PWD"
}
