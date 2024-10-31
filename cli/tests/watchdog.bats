#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for flox-watchdog
#
# bats file_tags=watchdog
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  wait_for_watchdogs "$PROJECT_DIR"
  project_teardown
  common_test_teardown
}

setup_sleeping_services() {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/sleeping_services.toml"
  assert_success
}

watchdog_pids_called_with_arg() {
  # This is a hack to essentially do a `pgrep` without having access to `pgrep`.
  # The `ps` prints `<pid> <cmd>`, then we use two separate `grep`s so that the
  # grep command itself doesn't get listed when we search for the data dir.
  # The `cut` just extracts the PID.
  pattern="$1"
  # echo "PATTERN: $pattern" >&3
  ps_output="$(ps -eo pid,args)"
  # echo "PS: $ps_output" >&3
  watchdogs="$(echo "$ps_output" | grep flox-watchdog)"
  # echo "WATCHDOGS: $watchdogs" >&3
  matches="$(echo "$watchdogs" | grep "$pattern")"
  # echo "MATCHES: $matches" >&3
  # This is a load-bearing 'xargs', it strips leading/trailing whitespace that
  # trips up 'cut'
  pids="$(echo "$matches" | xargs | cut -d' ' -f1)"
  # echo "PIDS: $pids" >&3
  echo "$pids"
}

# TODO: not very DRY, but I just copied this into start_shuts_down_process_compose.sh
process_compose_pids_called_with_arg() {
  # This is a hack to essentially do a `pgrep` without having access to `pgrep`.
  # The `ps` prints `<pid> <cmd>`, then we use two separate `grep`s so that the
  # grep command itself doesn't get listed when we search for the data dir.
  # The `cut` just extracts the PID.
  pattern="$1"
  ps_output="$(ps -eo pid,args)"
  process_composes="$(echo "$ps_output" | grep process-compose)"
  matches="$(echo "$process_composes" | grep "$pattern")"
  # This is a load-bearing 'xargs', it strips leading/trailing whitespace that
  # trips up 'cut'
  pids="$(echo "$matches" | xargs | cut -d' ' -f1)"
  echo "$pids"
}

# ---------------------------------------------------------------------------- #

@test "watchdog: can run flox-watchdog" {
  run "$WATCHDOG_BIN" --help
  assert_success
}

@test "watchdog: lives as long as the activation" {
  setup_sleeping_services
  export -f watchdog_pids_called_with_arg
  SHELL="bash" run --separate-stderr "$FLOX_BIN" activate -- bash <(cat <<'EOF'

    # Ensure that the watchdog is still running
    times=0
    while true; do
      if [ "$times" -gt 100 ]; then
        exit 1
      fi
      pid="$(watchdog_pids_called_with_arg "$_FLOX_SERVICES_SOCKET")"
      if [ -n "${pid?}" ]; then
        echo "$pid"
        break
      fi
      times=$((times + 1))
      sleep 0.01
    done
EOF
)
  pid="$output"
  assert_success

  # Ensure that the watchdog has exited now
  times=0
  while true; do
    if [ "$times" -gt 100 ]; then
      exit 1
    fi
    if ! kill -0 "$pid"; then
      break
    fi
    times=$((times + 1))
    sleep 0.01
  done
}

@test "watchdog: emits heartbeat log to prevent garbage collection while running" {
  target_pid="$$"

  # sets _FLOX_ATTACH, _FLOX_ACTIVATION_STATE_DIR, _FLOX_ACTIVATION_ID
  to_eval="$("$FLOX_ACTIVATIONS_BIN" --runtime-dir "$BATS_TEST_TMPDIR" start-or-attach \
    --pid "$target_pid" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --store-path "$BATS_TEST_TMPDIR"
  )"
  eval "$to_eval"

  # Close fd3 in case we don't make it to the final `kill`. We don't care about
  # orphaned children for the purpose of this test; watchdog will cleanup after
  # itself when BATS exits.
  # https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --log-dir "$BATS_TEST_TMPDIR" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --runtime-dir "$BATS_TEST_TMPDIR" \
    --activation-id "$_FLOX_ACTIVATION_ID" \
    --socket does_not_exist 3>&- &

  watchdog_pid="$!"
  log_file="$BATS_TEST_TMPDIR/watchdog.${_FLOX_ACTIVATION_ID}.log"

  # Wait for initial log entry. Other entries will be printed later but we don't
  # want to wait that long.
  timeout 1s bash -c "
    while ! grep -qs 'still watching, woof woof' \"$log_file\"; do
      sleep 0.1
    done
  "

  # Signal to exit.
  run kill "$watchdog_pid"
  assert_success
}

@test "watchdog: exits on termination signal (SIGUSR1)" {
  # sets _FLOX_ATTACH, _FLOX_ACTIVATION_STATE_DIR, _FLOX_ACTIVATION_ID
  to_eval="$("$FLOX_ACTIVATIONS_BIN" --runtime-dir "$BATS_TEST_TMPDIR" start-or-attach \
    --pid "$$" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --store-path "$BATS_TEST_TMPDIR"
  )"
  eval "$to_eval"

  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --log-dir "$BATS_TEST_TMPDIR" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --runtime-dir "$BATS_TEST_TMPDIR" \
    --activation-id "$_FLOX_ACTIVATION_ID" \
    --socket does_not_exist &
  watchdog_pid="$!"

  # Make our watchdog query command available in subshells
  export -f watchdog_pids_called_with_arg

  # Wait for start.
  run timeout 1s bash <(cat <<'EOF'
    while true; do
      pid="$(watchdog_pids_called_with_arg "$BATS_TEST_TMPDIR")"
      if [ -n "${pid?}" ]; then
        break
      fi
      sleep 0.01
    done
EOF
)
  assert_success

  # Check running.
  run kill -s 0 "$watchdog_pid"
  assert_success

  # Signal to exit.
  run kill -s SIGUSR1 "$watchdog_pid"
  assert_success

  # Wait for exit.
  run timeout 1s bash <(cat <<'EOF'
    while true; do
      pid="$(watchdog_pids_called_with_arg "$BATS_TEST_TMPDIR")"
      if [ -z "${pid?}" ]; then
        break
      fi
      sleep 0.01
    done
EOF
)
  assert_success
}

@test "watchdog: exits on shutdown signal (SIGINT)" {
  target_pid="$$"

  # sets _FLOX_ATTACH, _FLOX_ACTIVATION_STATE_DIR, _FLOX_ACTIVATION_ID
  to_eval="$("$FLOX_ACTIVATIONS_BIN" --runtime-dir "$BATS_TEST_TMPDIR" start-or-attach \
    --pid "$target_pid" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --store-path "$BATS_TEST_TMPDIR"
  )"
  eval "$to_eval"

  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --log-dir "$BATS_TEST_TMPDIR" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --runtime-dir "$BATS_TEST_TMPDIR" \
    --activation-id "$_FLOX_ACTIVATION_ID" \
    --socket does_not_exist &

  watchdog_pid="$!"
  log_file="$BATS_TEST_TMPDIR/watchdog.${_FLOX_ACTIVATION_ID}.log"

  # Wait for start.
  timeout 1s bash -c "
    while ! grep -qs 'watchdog is on duty' \"$log_file\"; do
      sleep 0.1
    done
  "

  # Check running.
  run kill -s 0 "$watchdog_pid"
  assert_success

  # Signal to exit.
  run kill -s SIGINT "$watchdog_pid"
  assert_success

  # Wait for exit.
  timeout 1s bash -c "
    while kill -s 0 \"$watchdog_pid\"; do
      sleep 0.1
    done
  "
}

@test "watchdog: exits when provided PID isn't running" {
  # We need a test PID, but PIDs can be reused. There's also no delay on reusing
  # PIDs, so you can't create and kill a process to use its PID during that
  # make-believe no-reuse window. At best we can choose a random PID and skip
  # the test if something is already using it.
  test_pid=31415
  if kill -0 "$test_pid"; then
    skip "test PID is in use"
  fi

  # sets _FLOX_ATTACH, _FLOX_ACTIVATION_STATE_DIR, _FLOX_ACTIVATION_ID
  to_eval="$("$FLOX_ACTIVATIONS_BIN" --runtime-dir "$BATS_TEST_TMPDIR" start-or-attach \
    --pid "$test_pid" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --store-path "$BATS_TEST_TMPDIR"
  )"
  eval "$to_eval"

  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --log-dir "$BATS_TEST_TMPDIR" \
    --flox-env "$BATS_TEST_TMPDIR" \
    --runtime-dir "$BATS_TEST_TMPDIR" \
    --activation-id "$_FLOX_ACTIVATION_ID" \
    --socket does_not_exist &

  watchdog_pid="$!"
  log_file="$BATS_TEST_TMPDIR/watchdog.${_FLOX_ACTIVATION_ID}.log"

  # Wait for start.
  timeout 1s bash -c "
    while ! grep -qs 'starting' \"$log_file\"; do
      sleep 0.1
    done
  "

  # The watchdog should immediately exit, so wait for it to exit.
  timeout 1s bash -c "
    while kill -s 0 \"$watchdog_pid\"; do
      sleep 0.1
    done
  "
}

@test "watchdog: shuts down process-compose started by imperative start" {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    "$FLOX_BIN" services start
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success
  assert_output --partial "Service 'one' started."

  export -f process_compose_pids_called_with_arg
  # Wait in case the watchdog doesn't shut down process-compose immediately
  timeout 1s bash -c '
    while [ -n "$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")" ]; do
      sleep .1
    done
  '
}
