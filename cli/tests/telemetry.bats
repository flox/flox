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

# A flush sends only what is buffered, so once `reset-metrics` has deleted the
# buffers, no buffered event recorded under the old device ID can be sent.
@test "reset-metrics deletes the device ID, both event buffers and the send-telemetry log" {
  # Fill the buffer and write a send-telemetry log under the current ID.
  export _FLOX_FORCE_FLUSH_METRICS=true
  run "$FLOX_BIN" envs --active
  assert_success
  wait_for_telemetry_flush
  local old_uuid
  old_uuid="$(cat "$FLOX_DATA_DIR/metrics-uuid")"
  # Only older releases write the legacy buffer. Leave one as they would.
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  printf '{"subcommand":"envs","uuid":"%s"}\n' "$old_uuid" > "$FLOX_CACHE_DIR/metrics-events-v2.json"
  # Every file the reset deletes exists, so the checks below are not vacuous.
  run grep -lF "$old_uuid" "$FLOX_DATA_DIR/events-v2.json" "$FLOX_CACHE_DIR/metrics-events-v2.json"
  assert_success
  assert_line "$FLOX_DATA_DIR/events-v2.json"
  assert_line "$FLOX_CACHE_DIR/metrics-events-v2.json"
  assert [ -e "$FLOX_DATA_DIR/events-v2.lock" ]
  assert [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
  assert [ -s "$FLOX_CACHE_DIR/log/send-telemetry.log" ]

  # Flushing stays forced, so a spawned sender would recreate the log.
  run "$FLOX_BIN" reset-metrics
  assert_success

  run grep -rlF "$old_uuid" "$FLOX_DATA_DIR" "$FLOX_CACHE_DIR"
  assert_failure 1
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.lock" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
  refute [ -e "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
}

@test "first command after reset-metrics shows the notice and buffers only the new device ID" {
  run "$FLOX_BIN" envs --active
  assert_success
  local old_uuid
  old_uuid="$(cat "$FLOX_DATA_DIR/metrics-uuid")"

  run "$FLOX_BIN" reset-metrics
  assert_success

  run "$FLOX_BIN" envs --active
  assert_success
  assert_output --partial "FLOX_DISABLE_METRICS=true"
  local new_uuid
  new_uuid="$(cat "$FLOX_DATA_DIR/metrics-uuid")"
  assert [ "$new_uuid" != "$old_uuid" ]

  run jq -rs 'map(.device_id) | unique | .[]' "$FLOX_DATA_DIR/events-v2.json"
  assert_success
  assert_output "$new_uuid"
  run grep -rlF "$old_uuid" "$FLOX_DATA_DIR" "$FLOX_CACHE_DIR"
  assert_failure 1
}

# ---------------------------------------------------------------------------- #

# `setup` exports `FLOX_DISABLE_METRICS=false`, so every DO_NOT_TRACK test below
# also checks that DO_NOT_TRACK wins over an explicit opt-in.

@test "DO_NOT_TRACK disables telemetry over FLOX_DISABLE_METRICS=false" {
  export DO_NOT_TRACK=true
  export _FLOX_FORCE_FLUSH_METRICS=true
  run --separate-stderr "$FLOX_BIN" envs --active
  assert_success
  # The command's own message and no first-run notice.
  assert_equal "$stderr" "No active environments"
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/log/send-telemetry.log" ]

  run "$FLOX_BIN" config
  assert_success
  assert_line "disable_metrics = true"
}

@test "DO_NOT_TRACK=0 leaves telemetry on" {
  export DO_NOT_TRACK=0
  run "$FLOX_BIN" envs --active
  assert_success
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]
}

@test "DO_NOT_TRACK disables Sentry" {
  export FLOX_SENTRY_DSN="http://public@127.0.0.1:1/1"

  export DO_NOT_TRACK=false
  run "$FLOX_BIN" -vv envs --active
  assert_success
  assert_output --partial "Initializing Sentry"

  export DO_NOT_TRACK=true
  run "$FLOX_BIN" -vv envs --active
  assert_success
  refute_output --partial "Initializing Sentry"
}

@test "DO_NOT_TRACK disables Sentry in the activation executive" {
  # Keep the background upgrade check off the network.
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true
  export FLOX_SENTRY_DSN="http://public@127.0.0.1:1/1"

  # The suite sets `_FLOX_EXECUTIVE_VERBOSITY=3`, which logs Sentry setup.
  export DO_NOT_TRACK=false
  "$FLOX_BIN" init -d tracked
  run "$FLOX_BIN" activate -d tracked -- true
  assert_success
  wait_for_activations tracked || return 1
  run cat tracked/.flox/log/executive.*
  assert_success
  assert_output --partial "Initializing Sentry"

  export DO_NOT_TRACK=true
  "$FLOX_BIN" init -d untracked
  run "$FLOX_BIN" activate -d untracked -- true
  assert_success
  wait_for_activations untracked || return 1
  run cat untracked/.flox/log/executive.*
  assert_success
  refute_output --partial "Initializing Sentry"
}

@test "DO_NOT_TRACK reaches activations as FLOX_DISABLE_METRICS=true" {
  # Keep the background upgrade check off the network.
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true
  export DO_NOT_TRACK=true
  "$FLOX_BIN" init -d project

  run --separate-stderr "$FLOX_BIN" activate -d project -c 'printenv FLOX_DISABLE_METRICS'
  assert_success
  assert_output "true"
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
}

# A config that fails to parse turns telemetry off until Flox reports the
# error (see config-errors.bats). DO_NOT_TRACK must not change that.
@test "DO_NOT_TRACK with a setting that fails to parse creates no device ID" {
  export DO_NOT_TRACK=true
  export FLOX_SEARCH_LIMIT=abc
  export _FLOX_FORCE_FLUSH_METRICS=true
  mkdir "$BATS_TEST_TMPDIR/cwd"
  cd "$BATS_TEST_TMPDIR/cwd"
  run "$FLOX_BIN" envs --active
  assert_failure
  assert_output --partial "Could not parse config"
  refute_output --partial "disable_metrics"
  refute_telemetry_files

  run ls -A
  assert_success
  assert_output ""
}

@test "config --set disable_metrics false notes that DO_NOT_TRACK still applies" {
  export DO_NOT_TRACK=true
  run "$FLOX_BIN" config --set disable_metrics false
  assert_success
  assert_output --partial "DO_NOT_TRACK still disables telemetry."

  run "$FLOX_BIN" config
  assert_success
  assert_line "disable_metrics = true"
}
