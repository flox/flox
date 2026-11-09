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
  # Skip the immediate first send so these tests exercise buffer expiry.
  mkdir -p "$FLOX_DATA_DIR"
  touch "$FLOX_DATA_DIR/events-v2-first-send"
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

@test "quiet first run shows the notice once and records telemetry" {
  unset RUST_LOG
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true

  run --separate-stderr "$FLOX_BIN" -q envs --active
  assert_success
  assert_regex "$stderr" "disable_metrics"
  refute_output --partial "disable_metrics"
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]
  assert [ -s "$FLOX_CACHE_DIR/metrics-events-v2.json" ]

  local uuid events
  uuid="$(cat "$FLOX_DATA_DIR/metrics-uuid")"
  events="$(wc -l < "$FLOX_DATA_DIR/events-v2.json")"
  run --separate-stderr "$FLOX_BIN" -q envs --active
  assert_success
  assert_equal "$stderr" ""
  assert_equal "$(cat "$FLOX_DATA_DIR/metrics-uuid")" "$uuid"
  assert [ "$(wc -l < "$FLOX_DATA_DIR/events-v2.json")" -gt "$events" ]
}

@test "first-run notice bypasses RUST_LOG filters" {
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true

  run --separate-stderr env RUST_LOG=off,flox::utils::message::unconditional=off "$FLOX_BIN" envs --active
  assert_success
  assert_regex "$stderr" "disable_metrics"
  refute_output --partial "disable_metrics"
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
  assert [ -s "$FLOX_DATA_DIR/events-v2.json" ]
}

@test "shell completion leaves the first-run notice for the next command" {
  unset RUST_LOG
  export _FLOX_TESTING_DISABLE_BG_SIDE_EFFECTS=true

  run --separate-stderr "$FLOX_BIN" --bpaf-complete-rev=8 en
  assert_success
  assert_output --partial "envs"
  assert_equal "$stderr" ""
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]

  run "$FLOX_BIN" envs --active
  assert_success
  assert_output --partial "disable_metrics"
  assert [ -s "$FLOX_DATA_DIR/metrics-uuid" ]
}

@test "metrics opt-out suppresses the first-run notice even in quiet mode" {
  run env FLOX_DISABLE_METRICS=true "$FLOX_BIN" -q envs --active
  assert_success
  refute_output --partial "disable_metrics"
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
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

refute_telemetry_files() {
  refute [ -e "$FLOX_DATA_DIR/metrics-uuid" ]
  refute [ -e "$FLOX_DATA_DIR/events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-events-v2.json" ]
  refute [ -e "$FLOX_CACHE_DIR/metrics-lock" ]
  refute [ -e "$FLOX_CACHE_DIR/log/send-telemetry.log" ]
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
  assert_output --partial "Unset DO_NOT_TRACK or set it to 0 to turn telemetry on."

  run "$FLOX_BIN" config
  assert_success
  assert_line "disable_metrics = true"
}

# Negative controls: the note appears only when DO_NOT_TRACK overrides the
# setting that was just written.
@test "config --set disable_metrics false does not mention DO_NOT_TRACK when it is not set" {
  export DO_NOT_TRACK=0
  run "$FLOX_BIN" config --set disable_metrics false
  assert_success
  refute_output --partial "DO_NOT_TRACK still disables telemetry."
}

@test "config --set disable_metrics true does not mention DO_NOT_TRACK" {
  export DO_NOT_TRACK=true
  run "$FLOX_BIN" config --set disable_metrics true
  assert_success
  refute_output --partial "DO_NOT_TRACK still disables telemetry."
}

# `flox <name>` runs a `flox-<name>` extension before the command line is
# parsed, so the extension would not get `FLOX_DISABLE_METRICS=true` from the
# code that exports it for the commands that Flox runs.
setup_extension_printing_opt_out() {
  export FLOX_FEATURES_BETA=true
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  cat > "$BATS_TEST_TMPDIR/bin/flox-optout" <<'SCRIPT'
#!/usr/bin/env bash
echo "FLOX_DISABLE_METRICS=${FLOX_DISABLE_METRICS-unset}"
SCRIPT
  chmod +x "$BATS_TEST_TMPDIR/bin/flox-optout"
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
}

@test "extension: DO_NOT_TRACK reaches the extension as FLOX_DISABLE_METRICS=true" {
  setup_extension_printing_opt_out
  export DO_NOT_TRACK=true
  run --separate-stderr "$FLOX_BIN" optout
  assert_success
  assert_output "FLOX_DISABLE_METRICS=true"
}

@test "extension: disable_metrics in a config file reaches the extension" {
  setup_extension_printing_opt_out
  unset FLOX_DISABLE_METRICS
  "$FLOX_BIN" config --set disable_metrics true
  run --separate-stderr "$FLOX_BIN" optout
  assert_success
  assert_output "FLOX_DISABLE_METRICS=true"
}

@test "extension: with telemetry on the extension gets no opt-out" {
  setup_extension_printing_opt_out
  unset FLOX_DISABLE_METRICS
  run --separate-stderr "$FLOX_BIN" optout
  assert_success
  assert_output "FLOX_DISABLE_METRICS=unset"
}
