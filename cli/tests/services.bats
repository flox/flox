#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Tests for service management
#
# bats file_tags=services
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
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
  project_teardown
  common_test_teardown
}

setup_sleeping_services() {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/sleeping_services.toml"
  assert_success
}

# ---------------------------------------------------------------------------- #
#
# NOTE: The following functionality is tested elsewhere:
#
#   - logs: providers/services.rs
#   - status: providers/services.rs
#   - remote environments: tests/environment-remotes.bats
#
# ---------------------------------------------------------------------------- #

@test "feature flag works" {
  RUST_LOG=flox=debug run "$FLOX_BIN" init
  refute_output --partial "service management enabled"
  unset output
  "$FLOX_BIN" delete -f
  RUST_LOG=flox=debug FLOX_FEATURES_SERVICES=true run "$FLOX_BIN" init
  assert_output --partial "service management enabled"
}

@test "can call process-compose" {
  run "$PROCESS_COMPOSE_BIN" version
  assert_success
  assert_output --partial "v1.9"
}

@test "process-compose can run generated config file" {
  export FLOX_FEATURES_SERVICES=true
  "$FLOX_BIN" init
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
  assert_success
  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
EOF
)
  assert_success
  [ -e hello.txt ]
}

@test "'flox activate -s' error without feature flag" {
  export FLOX_FEATURES_SERVICES=false
  "$FLOX_BIN" init
  manifest_file="${TESTS_DIR}/services/touch_file.toml"
  run "$FLOX_BIN" edit -f "$manifest_file"
  assert_success
  unset output
  run "$FLOX_BIN" activate -s
  assert_failure
  assert_output --partial "Services are not enabled in this environment"
}

@test "can start redis-server and access it using redis-cli" {
  export FLOX_FEATURES_SERVICES=true

  run "$FLOX_BIN" init
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/redis.json" \
    run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/redis.toml"
  assert_success

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    timeout 2s bash -c '
      while ! redis-cli -p "${REDIS_PORT}" ping; do
        sleep 0.1
      done
    '
EOF
)
  assert_success
  assert_output --partial "PONG"
}


# ---------------------------------------------------------------------------- #

# bats test_tags=services:stop
@test "stop: can't be used without feature flag" {
  run "$FLOX_BIN" services stop
  assert_failure
  assert_output "❌ ERROR: services are not enabled"
}

# bats test_tags=services:stop
@test "stop: errors if a service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' not found"
}

# bats test_tags=services:stop
@test "stop: errors before stopping if any service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop one invalid || exit_code=$?
    "$FLOX_BIN" services status
    exit $exit_code
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' not found"
  assert_output --regexp "one +Running"
  assert_output --regexp "two +Running"
}

# bats test_tags=services:stop
@test "stop: errors without stopping any services if preceeding service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop invalid one || exit_code=$?
    "$FLOX_BIN" services status
    exit $exit_code
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' not found"
  assert_output --regexp "one +Running"
  assert_output --regexp "two +Running"
}

# bats test_tags=services:stop
@test "stop: errors if service socket isn't responding" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    export _FLOX_SERVICES_SOCKET=invalid
    "$FLOX_BIN" services stop one invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: couldn't connect to service manager"
}

# bats test_tags=services:stop
@test "stop: stops all services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop
    "$FLOX_BIN" services status
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --partial "✅ Service 'two' stopped"
  assert_output --regexp "one +Completed"
  assert_output --regexp "two +Completed"
}

# bats test_tags=services:stop
@test "stop: stops a single service" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop one
    "$FLOX_BIN" services status
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --regexp "one +Completed"
  assert_output --regexp "two +Running"
}

# bats test_tags=services:stop
@test "stop: stops multiple services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop one two
    "$FLOX_BIN" services status
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --partial "✅ Service 'two' stopped"
  assert_output --regexp "one +Completed"
  assert_output --regexp "two +Completed"
}

# bats test_tags=services:stop
@test "stop: errors if service is already stopped" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop one
    "$FLOX_BIN" services status
    "$FLOX_BIN" services stop one
EOF
)
  assert_success
  assert_output --regexp "one +Completed"
  assert_output --partial "⚠️  Service 'one' is not running"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:status
@test "status: lists the statuses for services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services status
EOF
)
  assert_success
  assert_output --regexp "NAME +STATUS +PID"
  assert_output --regexp "one +Running +[0-9]+"
  assert_output --regexp "two +Running +[0-9]+"
}

# ---------------------------------------------------------------------------- #

@test "activate services: shows warning when services already running" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  dummy_socket="$PWD/sock.sock"
  touch "$dummy_socket"
  _FLOX_SERVICES_SOCKET="$dummy_socket" run "$FLOX_BIN" activate -s -- true

  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"
}

# ---------------------------------------------------------------------------- #

@test "blocking: error message when startup times out" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  export _FLOX_SERVICES_ACTIVATE_TIMEOUT=0.1
  export _FLOX_SERVICES_LOG_FILE="$PROJECT_DIR/logs.txt"
  # process-compose will never be able to create this socket,
  # which looks the same as taking a long time to create the socket
  export _FLOX_SERVICES_SOCKET="/no_permission.sock"
  # As of version 1.6.1, there's a race condition in process-compose such that
  # it may leave behind a sleep process.
  # Close FD 3 so bats doesn't hang forever.
  # Kill sleep for now just to be safe.

  run "$FLOX_BIN" activate -s -- true
  assert_output --partial "❌ Failed to start services"
}

@test "blocking: activation blocks on socket creation" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  export _FLOX_SERVICES_LOG_FILE="$PROJECT_DIR/logs.txt"
  # This is run immediately after activation starts, which is about as good
  # as we can get for checking that activation has blocked until the socket
  # exists
  run "$FLOX_BIN" activate -s -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$PROCESS_COMPOSE_BIN" process list
EOF
)
  # Just assert that one of our processes shows up in the output, which indicates
  # that process-compose has responded
  assert_output --partial "flox_never_exit"
}

@test "blocking: process-compose writes logs to file" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  export _FLOX_SERVICES_LOG_FILE="$PROJECT_DIR/logs.txt"
  "$FLOX_BIN" activate -s -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    # No actual work to do here other than let process-compose
    # start and write to logs
EOF
)
  # Check that a startup log line shows up in the logs
  run grep "process=flox_never_exit" "$_FLOX_SERVICES_LOG_FILE"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "watchdog: can run klaus" {
  run "$KLAUS_BIN" --help
  assert_success
}

@test "watchdog: lives as long as the activation" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services
  run "$FLOX_BIN" activate -s -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

    log_file="$PWD/.flox/cache/$(ls .flox/cache)"

    # Ensure that the watchdog is still running
    if tail -n 1 "$log_file" | grep exiting; then
      cat "$log_file" >&3
      exit 1
    fi
EOF
)
  assert_success

  # Ensure that the watchdog has exited now
  log_file="$PWD/.flox/cache/$(ls .flox/cache)"
  if ! tail -n 1 "$log_file" | grep exiting; then
    cat "$log_file" >&3
    exit 1
  fi
  assert_success
}

@test "watchdog: exits on termination signal (SIGUSR1)" {
  log_file=klaus.log
  registry_file=registry.json
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$KLAUS_BIN" \
    --logs "$log_file" \
    --pid $$ \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &
  klaus_pid="$!"

  # Wait for start.
  timeout 1s bash -c "
    while ! grep -qs 'watchdog is on duty' \"$log_file\"; do
      sleep 0.1
    done
  "

  # Check running.
  run kill -s 0 "$klaus_pid"
  assert_success

  # Signal to exit.
  run kill -s SIGUSR1 "$klaus_pid"
  assert_success

  # Wait for exit.
  timeout 1s bash -c "
    while kill -s 0 \"$klaus_pid\"; do
      sleep 0.1
    done
  "
}

@test "watchdog: exits on shutdown signal (SIGINT)" {
  log_file=klaus.log
  registry_file=registry.json
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$KLAUS_BIN" \
    --logs "$log_file" \
    --pid $$ \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &
  klaus_pid="$!"

  # Wait for start.
  timeout 1s bash -c "
    while ! grep -qs 'watchdog is on duty' \"$log_file\"; do
      sleep 0.1
    done
  "

  # Check running.
  run kill -s 0 "$klaus_pid"
  assert_success

  # Signal to exit.
  run kill -s SIGINT "$klaus_pid"
  assert_success

  # Wait for exit.
  timeout 1s bash -c "
    while kill -s 0 \"$klaus_pid\"; do
      sleep 0.1
    done
  "
}

@test "watchdog: exits when provided PID isn't running" {
  log_file=klaus.log

  # We need a test PID, but PIDs can be reused. There's also no delay on reusing
  # PIDs, so you can't create and kill a process to use its PID during that
  # make-believe no-reuse window. At best we can choose a random PID and skip
  # the test if something is already using it.
  test_pid=31415
  if kill -0 "$test_pid"; then
    skip "test PID is in use"
  fi

  registry_file=registry.json
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$KLAUS_BIN" \
    --logs "$log_file" \
    --pid "$test_pid" \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &
  klaus_pid="$!"

  # Wait for start.
  timeout 1s bash -c "
    while ! grep -qs 'starting' \"$log_file\"; do
      sleep 0.1
    done
  "

  # The watchdog should immediately exit, so wait for it to exit.
  timeout 1s bash -c "
    while kill -s 0 \"$klaus_pid\"; do
      sleep 0.1
    done
  "
}
