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

# Wait, with a poll and timeout, for a file to match some contents.
#
# This can be used to prevent race conditions where we expect something to
# happen _at least_ N times.
wait_for_file_content() {
  file="$1"
  expected="$2"

  run timeout 1s bash -c '
    while [ "$(cat '$file')" != "'$expected'" ]; do
      sleep 0.1s
    done
  '
  assert_success
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

setup_logging_services() {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/logging_services.toml"
  assert_success
}

setup_start_counter_services() {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/start_counter_services.toml"
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

@test "can call process-compose" {
  run "$PROCESS_COMPOSE_BIN" version
  assert_success
  assert_output --partial "v1.9"
}

@test "process-compose can run generated config file" {
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

@test "can start redis-server and access it using redis-cli" {

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

@test "services aren't started unless requested" {
  setup_sleeping_services

  RUST_LOG=debug run "$FLOX_BIN" activate -- true
  assert_output --partial "start=false"
  assert_output --partial "will not start services"
}

@test "all imperative commands error when no services are defined" {
  run "$FLOX_BIN" init

  commands=("logs" "restart" "start" "status" "stop")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"
    # NB: No --start-services.
    run "$FLOX_BIN" activate -- "$FLOX_BIN" services "$command"
    assert_failure
    assert_line "❌ ERROR: Environment doesn't have any services defined."
  done
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:restart
@test "restart: errors before restarting if any service doesn't exist" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services restart one two invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' not found"

  # This doesn't guarantee that the services haven't been restarted _after_
  # we've read the counter files. So an intermittent failure could indicate that
  # our error handling is wrong or that the behaviour of `process-compose` has
  # changed.
  wait_for_file_content start_counter.one 1
  wait_for_file_content start_counter.two 1
  wait_for_file_content start_counter.sleeping 1
}

# bats test_tags=services:restart
@test "restart: errors when used outside an activation" {
  setup_start_counter_services

  run "$FLOX_BIN" services restart one
  assert_failure
  assert_line "❌ ERROR: Cannot restart services for an environment that is not activated."
}

# bats test_tags=services:restart
@test "restart: restarts a single service" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    # Wait for completion so that we indicate "start" instead of "restart"
    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed
    "$FLOX_BIN" services restart one
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"

  wait_for_file_content start_counter.one 2
  wait_for_file_content start_counter.two 1
  wait_for_file_content start_counter.sleeping 1
}

# bats test_tags=services:restart
@test "restart: restarts multiple services" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    # Wait for completion so that we indicate "start" instead of "restart"
    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed two:Completed
    "$FLOX_BIN" services restart one two
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"
  assert_output --partial "✅ Service 'two' started"

  wait_for_file_content start_counter.one 2
  wait_for_file_content start_counter.two 2
  wait_for_file_content start_counter.sleeping 1
}

# bats test_tags=services:restart
@test "restart: restarts all services (incl. running and completed)" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    # Wait for completion so that we indicate "start" instead of "restart"
    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed two:Completed sleeping:Running
    "$FLOX_BIN" services restart
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"
  assert_output --partial "✅ Service 'two' started"
  assert_output --partial "✅ Service 'sleeping' restarted"

  wait_for_file_content start_counter.one 2
  wait_for_file_content start_counter.two 2
  wait_for_file_content start_counter.sleeping 2
}

# bats test_tags=services:restart
@test "restart: restarts stopped services" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop sleeping
    "$FLOX_BIN" services restart sleeping
EOF
)
  assert_success
  assert_output --partial "✅ Service 'sleeping' stopped"
  assert_output --partial "✅ Service 'sleeping' started"

  wait_for_file_content start_counter.sleeping 2
}

# bats test_tags=services:restart
@test "restart: starts a specified service when activation hasn't already started services" {
  setup_start_counter_services

  # NB: No --start-services.
  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services restart one
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"
  refute_output --partial "Service 'two'"
  refute_output --partial "Service 'sleeping'"

  # Can't reliably assert that the other services didn't start.
  wait_for_file_content start_counter.one 1
}

# bats test_tags=services:restart
@test "restart: status still works when activation (re)starts a single shortlived service" {

  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
  assert_success

  # NB: No --start-services.
  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services restart touch_file
    "$FLOX_BIN" services status
EOF
)
  assert_success
  assert_output --partial "✅ Service 'touch_file' started"
  assert_output --regexp "touch_file +(Running|Completed)"
}

# bats test_tags=services:restart
@test "restart: starts all services when activation hasn't already started services" {
  setup_start_counter_services

  # NB: No --start-services.
  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services restart
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"
  assert_output --partial "✅ Service 'two' started"
  assert_output --partial "✅ Service 'sleeping' started"

  wait_for_file_content start_counter.one 1
  wait_for_file_content start_counter.two 1
  wait_for_file_content start_counter.sleeping 1
}

# bats test_tags=services:restart
@test "restart: does not reload config when some services are still running" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    # Wait for completion so that we indicate "start" instead of "restart"
    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart one
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' started"
  refute_output --partial "Service 'two'"
  refute_output --partial "Service 'sleeping'"
  refute_output --partial "Service 'touch_file'"

  wait_for_file_content start_counter.one 2
}

# bats test_tags=services:restart
@test "restart: reloads config when all services are restarted" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart
EOF
)
  assert_success
  assert_output --partial "✅ Service 'touch_file' started"
  [ -e hello.txt ]
}

# bats test_tags=services:restart
@test "restart: reloads config when given no service and all services are stopped" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart
EOF
)
  assert_success
  assert_output --partial "✅ Service 'touch_file' started"
  [ -e hello.txt ]
}

# bats test_tags=services:restart
@test "restart: reloads config when given single service and all services are stopped" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart touch_file
EOF
)
  assert_success
  assert_output --partial "✅ Service 'touch_file' started"
  [ -e hello.txt ]
}

# bats test_tags=services:restart
@test "restart: errors when given service isn't in reloaded config" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart one
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'one' not found"
  refute_output --partial "Service 'touch_file'"
  [ ! -e hello.txt ]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:stop
@test "stop: errors if a service doesn't exist" {
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
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services stop one two invalid || exit_code=$?
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

# bats test_tags=services:logs:tail:exactly-one-service
@test "logs: tail: requires exactly one service" {
  setup_logging_services
  run "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services logs one
EOF
  )
  assert_success
}

# bats test_tags=services:logs:tail:exactly-one-service
@test "logs: tail: requires exactly one service - error on multiple services" {
  setup_logging_services

  # try running with multiple services specified
  run "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services logs one two
EOF
  )
  assert_failure
  assert_line "❌ ERROR: A single service name is required when the --follow flag is not specified"
}

# bats test_tags=services:logs:tail:exactly-one-service
@test "logs: tail: requires exactly one service - error without services" {
  setup_logging_services

  # Try running without services specified
  run "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services logs
EOF
  )
  assert_failure
  assert_line "❌ ERROR: A single service name is required when the --follow flag is not specified"
}

# bats test_tags=services:logs:tail:no-such-service
@test "logs: tail: requires exactly one service - error if service doesn't exist" {
  setup_logging_services

  # Try running with a nonexisting services specified
  run "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"
    "$FLOX_BIN" services logs doesnotexist
EOF
  )
  assert_failure
  assert_line "❌ ERROR: Service 'doesnotexist' not found."
}

# Runs a service that will sleep after printing a few lines of logs.
# Assert that flox is _not_ waiting for the service to finish.
# bats test_tags=services:logs:tail:instant
@test "logs: tail does not wait" {
  setup_logging_services

  run --separate-stderr "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

    timeout 1 "$FLOX_BIN" services logs mostly-deterministic
EOF
  )

  assert_success
  assert_output - <<EOF
1
2
3
EOF
}

# ---------------------------------------------------------------------------- #

# NOTE: this test will wait out the sleep in the `mostly-deterministic` service.
# We generally avoid sleeping and exit as quickly as possible!
# This is an exception to explicitly test the blocking behavior of `logs --follow`
# bats test_tags=services:logs:follow:blocks
@test "logs: follow will wait for logs" {
  setup_logging_services

  # We expect flox to block and be killed by `timeout`
  run -124 --separate-stderr "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

    # At the time of writing, the `mostly-deterministic` service sleeps for 3 seconds
    # Give flox a 4 second timeout to ensure the service has time to wake and log.
    timeout 4 "$FLOX_BIN" services logs --follow mostly-deterministic
EOF
  )

  assert_output - <<EOF
mostly-deterministic: 1
mostly-deterministic: 2
mostly-deterministic: 3
mostly-deterministic: 4
EOF
}

# bats test_tags=services:logs:follow:combines
@test "logs: follow shows logs for multiple services" {
  setup_logging_services

  mkfifo ./resume-one.pipe
  mkfifo ./resume-mostly-deterministic.pipe

  # We expect flox to block and be killed by `timeout`, which will return a 124 exit code
  run -124 --separate-stderr "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

    # ensure some logs are printed for both services then stop the log reader
    # both processes write to the pipe once to signal they have written _something_
    # (they will also _wait_ until the pipe is read)
    read < ./resume-one.pipe
    read < ./resume-mostly-deterministic.pipe

    # kill log reading, because with `--follow` the process wil block indefinitely
    timeout 0.5 "$FLOX_BIN" services logs --follow one mostly-deterministic
EOF
  )

  assert_line --regexp "^mostly-deterministic: "
  assert_line --regexp "^one                 : "
}

# bats test_tags=services:logs:follow:combines
@test "logs: follow shows logs for all services if no names provided" {
  setup_logging_services

  mkfifo ./resume-one.pipe
  mkfifo ./resume-mostly-deterministic.pipe

  # We expect flox to block and be killed by `timeout`
  "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

    # ensure some logs are printed for both services then stop the log reader
    # both processes write to the pipe once to signal they have written _something_
    # (they will also _wait_ until the pipe is read)
    read < ./resume-one.pipe
    read < ./resume-mostly-deterministic.pipe

    "$FLOX_BIN" services logs --follow > logs &
    logs_pid="$!"

    for i in {1..10}; do
      if grep "^mostly-deterministic: " logs && grep "^one                 : " logs; then
        break
      fi
      sleep .1
    done
    if [ "$i" -eq 10 ]; then
      echo "didn't find expected logs"
      # kill log reading, because with `--follow` the process wil block indefinitely
      kill -SIGTERM "$logs_pid"
      exit 1
    fi

    # kill log reading, because with `--follow` the process wil block indefinitely
    kill -SIGTERM "$logs_pid"
EOF
  )
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:status
@test "status: lists the statuses for services" {
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
  setup_sleeping_services
  dummy_socket="$PWD/sock.sock"
  touch "$dummy_socket"
  _FLOX_SERVICES_SOCKET="$dummy_socket" run "$FLOX_BIN" activate -s -- true

  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"
}

# ---------------------------------------------------------------------------- #

@test "blocking: error message when startup times out" {
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

@test "watchdog: can run flox-watchdog" {
  run "$WATCHDOG_BIN" --help
  assert_success
}

@test "watchdog: lives as long as the activation" {
  setup_sleeping_services
  export -f watchdog_pids_called_with_arg
  SHELL="bash" run --separate-stderr "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/register_cleanup.sh"

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

@test "watchdog: exits on termination signal (SIGUSR1)" {
  # Don't forget to export this so that it's set in the subshells
  export registry_file="$PWD/registry.json"

  log_file="$PWD/flox-watchdog.log"
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --logs "$log_file" \
    --pid $$ \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &
  watchdog_pid="$!"

  # Make our watchdog query command available in subshells
  export -f watchdog_pids_called_with_arg

  # Wait for start.
  run timeout 1s bash <(cat <<'EOF'
    while true; do
      pid="$(watchdog_pids_called_with_arg "$registry_file")"
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
      pid="$(watchdog_pids_called_with_arg "$registry_file")"
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
  # Don't forget to export this so that it's set in the subshells
  export log_file="$PWD/flox-watchdog.log"

  registry_file="$PWD/registry.json"
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --logs "$log_file" \
    --pid $$ \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &

  # Don't forget to export this so that it's set in the subshells
  export watchdog_pid="$!"

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
  # Don't forget to export this so that it's set in the subshells
  export log_file="$PWD/flox-watchdog.log"

  # We need a test PID, but PIDs can be reused. There's also no delay on reusing
  # PIDs, so you can't create and kill a process to use its PID during that
  # make-believe no-reuse window. At best we can choose a random PID and skip
  # the test if something is already using it.
  test_pid=31415
  if kill -0 "$test_pid"; then
    skip "test PID is in use"
  fi

  registry_file="$PWD/registry.json"
  dummy_registry path/to/env abcde123 > "$registry_file"
  _FLOX_WATCHDOG_LOG_LEVEL=debug "$WATCHDOG_BIN" \
    --logs "$log_file" \
    --pid "$test_pid" \
    --registry "$registry_file" \
    --hash abcde123 \
    --socket does_not_exist &

  # Don't forget to export this so that it's set in the subshells
  export watchdog_pid="$!"

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

@test "start: errors if service doesn't exist" {

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    # don't set -euo pipefail because we expect these to fail
    "$FLOX_BIN" services start one invalid
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_failure
  assert_output --partial "Service 'invalid' not found."
  assert_output --partial "couldn't connect to service manager"
}

# Also tests service names with spaces in them, because starting them is handled
# in Bash
@test "start: only starts specified services" {


  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    no_space.command = "sleep infinity"
    "with space".command = "sleep infinity"
    skip.command = "sleep infinity"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    "$FLOX_BIN" services start no_space "with space"
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success
  assert_output --partial "Service 'no_space' started."
  assert_output --partial "Service 'with space' started."
  assert_output --partial "no_space   Running"
  assert_output --partial "with space Running"
  assert_output --partial "skip       Disabled"
}

@test "start: defaults to all services" {


  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
    two.command = "sleep infinity"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    "$FLOX_BIN" services start
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success
  assert_output --partial "Service 'one' started."
  assert_output --partial "Service 'two' started."
  assert_output --partial "one        Running"
  assert_output --partial "two        Running"
}

@test "start: status still works when activation starts a single shortlived service" {

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo done"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    "$FLOX_BIN" services start one
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success
  assert_output --partial "Service 'one' started."
  assert_output --regexp "one +(Running|Completed)"
}

@test "start: picks up changes after environment modification when all services have stopped" {


  MANIFEST_CONTENTS_1="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo $FOO"

    [hook]
    on-activate = "export FOO=foo_one"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS_1" | "$FLOX_BIN" edit -f -

  # Edit the manifest adding a second service and changing the value of FOO.
  # Then start services again.
  run "$FLOX_BIN" activate -s -- bash "${TESTS_DIR}/services/start_picks_up_modifications.sh"
  assert_success

  # The added service should be running.
  assert_output --partial "two        Running"
  # The updated value of FOO should be printed
  assert_output --partial "foo_two"
}

@test "start: does not pick up changes after environment modification when some services still running" {


  MANIFEST_CONTENTS_1="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS_1" | "$FLOX_BIN" edit -f -

  # Edit the manifest adding a second service.
  # Then try to start the second service.
  run "$FLOX_BIN" activate -s -- bash "${TESTS_DIR}/services/start_does_not_pick_up_modifications.sh"
  assert_failure
  assert_output --partial "Service 'two' not found."
}


@test "start: shuts down existing process-compose" {

  MANIFEST_CONTENTS_1="$(cat << "EOF"
    version = 1

    [services]
    one.command = "true"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS_1" | "$FLOX_BIN" edit -f -

  # Call flox services start and check if the prior process-compose gets shutdown
  # This also appears to hang forever if process-compose doesn't get shutdown
  run "$FLOX_BIN" activate -s -- bash "${TESTS_DIR}/services/start_shuts_down_process_compose.sh"
  assert_success
}


@test "start: watchdog shuts down process-compose started by start" {

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

  # Wait in case the watchdog doesn't shut down process-compose immediately
  for i in {1..5}; do
    if [ -z "$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")" ]; then
      break
    fi
    sleep .1
  done
  if [ "$i" -eq 5 ]; then
    echo "process-compose is still running"
    return 1
  fi
}

@test "kills daemon process" {

  MANIFEST_CONTENTS="$(cat <<"EOF"
    version = 1

    [install]
    overmind.pkg-path = "overmind"

    [services.overmind]
    command = "overmind start -D"
    is-daemon = true
    shutdown.command = "overmind quit"
EOF
)"

  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/overmind.json"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -
  echo "sleep: sleep 999999" > ./Procfile

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    sleep 0.1 # small delay to let the services start
    overmind status

    "$FLOX_BIN" services status
    "$FLOX_BIN" services stop
    sleep 0.1 # small delay to make sure services have stopped
EOF
  )"

  run "$FLOX_BIN" activate -s -- bash -c "$SCRIPT"
  assert_success
  run [ ! -e "$PWD/overmind.sock" ]
  assert_success
}
