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

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown() {
  # Wait for watchdogs before project teardown, otherwise some tests will hang
  # forever.
  #
  # I'm guessing this is because the watchdog and process-compose have logfiles
  # in the project directory,
  # so maybe one of them tries to log something and hangs.
  # ps output is showing a process-compose down hanging forever,
  # so that's a likely culprit.
  # See https://github.com/flox/flox/actions/runs/10820753745/job/30021432134#step:9:26
  # I'd check the logs to confirm what's happening...
  # ...if only the reproducer wasn't to delete the logs.
  #
  # When running in parallel `wait_for_watchdogs`
  # may wait for watchdog processes of unrelated tests.
  # It tries to avoid non-test processes by looking for the data dir argument,
  # passed to the watchdog process.
  # Within the `services` tests, we call `setup_isolated_flox` during `setup()`,
  # which sets the data dir to a unique value for every test,
  # thus avoiding waiting for unrelated watchdog processes.
  wait_for_watchdogs
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
#
# ---------------------------------------------------------------------------- #

@test "can call process-compose" {
  run "$PROCESS_COMPOSE_BIN" version
  assert_success
  assert_output --partial "v1.27.0"
}

@test "process-compose can run generated config file" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
  assert_success
  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
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
  assert_output --partial "setting service variables should_have_services=false start_new_process_compose=false"
}

@test "all imperative commands error when no services are defined" {
  run "$FLOX_BIN" init

  commands=("logs" "restart" "start" "status" "stop")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"
    # NB: No --start-services.
    run "$FLOX_BIN" activate -- "$FLOX_BIN" services "$command"
    assert_failure
    assert_line "❌ ERROR: Environment does not have any services defined."
  done
}

@test "all imperative commands error when no services are defined for the current system" {
  run "$FLOX_BIN" init

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
    one.systems = ["dummy-system"]
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -


  commands=("logs" "restart" "start" "status" "stop")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"
    # NB: No --start-services.
    run "$FLOX_BIN" activate -- "$FLOX_BIN" services "$command"
    assert_failure
    assert_line "❌ ERROR: Environment does not have any services defined for '$NIX_SYSTEM'."
  done
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:manifest-changes
@test "install: warns about restarting services" {
  setup_sleeping_services
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" install hello
EOF
)
  assert_success
  assert_line "⚠️  Your manifest has changes that may require running 'flox services restart'."
}

# bats test_tags=services:manifest-changes
@test "uninstall: warns about restarting services" {
  setup_sleeping_services
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" install hello

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" uninstall hello
EOF
)
  assert_success
  assert_line "⚠️  Your manifest has changes that may require running 'flox services restart'."
}

# bats test_tags=services:manifest-changes
@test "upgrade: warns about restarting services" {
  setup_sleeping_services
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.json" \
    run "$FLOX_BIN" install hello

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
      "$FLOX_BIN" upgrade
EOF
)
  assert_success
  assert_line "⚠️  Your manifest has changes that may require running 'flox services restart'."
}

# bats test_tags=services:manifest-changes
@test "edit: warns about restarting services" {
  setup_sleeping_services
  cat > manifest.toml << EOF
version = 1
EOF

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" edit -f manifest.toml
EOF
)
  assert_success
  assert_line "⚠️  Your manifest has changes that may require running 'flox services restart'."
}

# bats test_tags=services:manifest-changes
@test "pull: warns about restarting services" {
  export OWNER="owner"

  setup_isolated_flox
  setup_sleeping_services
  floxhub_setup "$OWNER"

  run "$FLOX_BIN" push --owner "$OWNER"
  assert_success

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    "$FLOX_BIN" install hello --remote "$OWNER/$PROJECT_NAME"
  assert_success

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" pull
EOF
)
  assert_success
  assert_line "⚠️  Your manifest has changes that may require running 'flox services restart'."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:restart
@test "restart: errors before restarting if any service doesn't exist" {
  setup_start_counter_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" services restart one two invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' does not exist."

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
    "$FLOX_BIN" services stop
    "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
    "$FLOX_BIN" services restart one
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'one' does not exist."
  refute_output --partial "Service 'touch_file'"
  [ ! -e hello.txt ]
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:stop
@test "stop: errors if a service doesn't exist" {
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "$FLOX_BIN" services stop invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' does not exist."
}

# bats test_tags=services:stop
@test "stop: errors before stopping if any service doesn't exist" {
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    "$FLOX_BIN" services stop one two invalid || exit_code=$?
    "$FLOX_BIN" services status
    exit $exit_code
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: Service 'invalid' does not exist."
  assert_output --regexp "one +Running"
  assert_output --regexp "two +Running"
}

# bats test_tags=services:detect-not-started
@test "services: errors if services are not started" {
  setup_sleeping_services

  commands=("logs" "status" "stop")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"

    command="$command" run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
      rm -f "$_FLOX_SERVICES_SOCKET"
      "$FLOX_BIN" services "$command" one invalid
EOF
)
    assert_failure
    assert_output --partial "❌ ERROR: Services not started or quit unexpectedly."
  done
}

# bats test_tags=services:permit-stopped-for-start-restart
@test "start, restart: do not require active services" {

  setup_sleeping_services

  commands=("start" "restart")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"
    command="$command" run "$FLOX_BIN" activate -- bash <(cat <<EOF

      [ ! -e "\$_FLOX_SERVICES_SOCKET" ] || exit 2

      "$FLOX_BIN" services "$command"
EOF
)
    assert_success

    # give the watchdog a chance to clean up the services before the next iteration
    wait_for_watchdogs
  done
}

# bats test_tags=services:stop
@test "stop: stops all services" {
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
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
    "$FLOX_BIN" services logs doesnotexist
EOF
  )
  assert_failure
  assert_line "❌ ERROR: Service 'doesnotexist' does not exist."
}

# Runs a service that will sleep after printing a few lines of logs.
# Assert that flox is _not_ waiting for the service to finish.
# bats test_tags=services:logs:tail:instant
@test "logs: tail does not wait" {
  setup_logging_services

  run --separate-stderr "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'

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

    # ensure some logs are printed for both services then stop the log reader
    # both processes write to the pipe once to signal they have written _something_
    # (they will also _wait_ until the pipe is read)
    read < ./resume-one.pipe
    read < ./resume-mostly-deterministic.pipe

    "$FLOX_BIN" services logs --follow > logs &
    logs_pid="$!"

    if timeout 1s bash -c '
      while ! grep "^mostly-deterministic: " logs || !grep "^one                 : " logs; do
        sleep .1
      done
    '; then
      # kill log reading, because with `--follow` the process wil block indefinitely
      kill -SIGTERM "$logs_pid"
    else
      echo "didn't find expected logs"
      # kill log reading, because with `--follow` the process wil block indefinitely
      kill -SIGTERM "$logs_pid"
      exit 1
    fi
EOF
  )
}

# ---------------------------------------------------------------------------- #

# bats test_tags=services:status
@test "status: lists the statuses for services" {
  setup_sleeping_services
  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
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
  mkfifo fifo
  "$FLOX_BIN" activate -s -- echo \> fifo &
  activate_pid="$!"
  # Make sure the first `process-compose` gets up and running
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Running

  run "$FLOX_BIN" activate -s -- true
  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"

  # Technically this should be a teardown step
  # The test will hang forever if it fails and doesn't get here
  timeout 2 cat fifo
}

# ---------------------------------------------------------------------------- #

@test "blocking: error message when startup times out" {
  setup_sleeping_services
  export _FLOX_SERVICES_ACTIVATE_TIMEOUT=0.1
  # process-compose will never be able to create this socket,
  # which looks the same as taking a long time to create the socket
  export _FLOX_SERVICES_SOCKET_OVERRIDE="/no_permission.sock"

  run "$FLOX_BIN" activate -s -- true
  assert_output --partial "❌ Failed to start services"
}

@test "blocking: activation blocks on process list" {
  setup_sleeping_services
  # This is run immediately after activation starts, which is about as good as
  # we can get for checking that activation has blocked until process list
  # succeeds
  run "$FLOX_BIN" activate -s -- bash <(cat <<'EOF'
    "$PROCESS_COMPOSE_BIN" process list -u $_FLOX_SERVICES_SOCKET
EOF
)
  # Just assert that one of our processes shows up in the output, which indicates
  # that process-compose has responded
  assert_output --partial "flox_never_exit"
}

@test "activate: child processes write logs to .flox/log" {
  setup_sleeping_services
  "$FLOX_BIN" activate -s -- bash <(cat <<'EOF'
    # No actual work to do here other than let process-compose
    # start and write to logs
EOF
)

  # Check that a startup log line shows up in the logs
  run grep "process=flox_never_exit" "${PROJECT_DIR}"/.flox/log/services.*.log
  assert_success

  # Poll because watchdog may not have started by the time the activation finishes.
  run timeout 1s bash -c "
    while ! grep 'flox_watchdog: starting' \"$PROJECT_DIR\"/.flox/log/watchdog.*.log; do
      sleep 0.1
    done
  "
  assert_success
}

@test "activate: --start-services warns if environment does not have services" {
  run "$FLOX_BIN" init
  assert_success

  run "$FLOX_BIN" activate --start-services -- true
  assert_success
  assert_output "⚠️  Environment does not have any services defined."
}

@test "activate: outer activation starts services and inner activation doesn't" {
  setup_sleeping_services

  export INNER_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$INNER_PROJECT_DIR"
  assert_success

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    "${TESTS_DIR}/services/echo_activate_vars.sh" outer
    "$FLOX_BIN" activate -d "$INNER_PROJECT_DIR" -- bash "${TESTS_DIR}/services/echo_activate_vars.sh" inner
EOF
)
  assert_success
  assert_line "outer FLOX_ACTIVATE_START_SERVICES=true"
  assert_line "outer _FLOX_SERVICES_TO_START=unset"
  assert_line "inner FLOX_ACTIVATE_START_SERVICES=false"
  assert_line "inner _FLOX_SERVICES_TO_START=unset"
}

@test "activate: outer activation imperatively starts services and inner activation doesn't" {
  setup_sleeping_services

  export INNER_PROJECT_DIR="${PROJECT_NAME}-nested"
  "$FLOX_BIN" init -d "$INNER_PROJECT_DIR"
  assert_success

  # NB: no --start-services
  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    "$FLOX_BIN" services start
    "${TESTS_DIR}/services/echo_activate_vars.sh" outer
    "$FLOX_BIN" activate -d "$INNER_PROJECT_DIR" -- bash "${TESTS_DIR}/services/echo_activate_vars.sh" inner
EOF
)
  assert_success
  assert_line "outer FLOX_ACTIVATE_START_SERVICES=false"
  assert_line "outer _FLOX_SERVICES_TO_START=unset"
  assert_line "inner FLOX_ACTIVATE_START_SERVICES=false"
  assert_line "inner _FLOX_SERVICES_TO_START=unset"
}

@test "activate: services can be layered" {

  MANIFEST_CONTENTS_1="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo one"
EOF
  )"

  "$FLOX_BIN" init -d one
  echo "$MANIFEST_CONTENTS_1" | "$FLOX_BIN" edit -d one -f -


  MANIFEST_CONTENTS_2="$(cat << "EOF"
    version = 1

    [services]
    two.command = "echo two"
EOF
  )"

  "$FLOX_BIN" init -d two
  echo "$MANIFEST_CONTENTS_2" | "$FLOX_BIN" edit -d two -f -

  cat <<"EOF" > script_inner.sh
    set -euo pipefail


    "${TESTS_DIR}"/services/wait_for_service_status.sh two:Completed
EOF

  "$FLOX_BIN" activate -d one --start-services -- bash <(cat <<"EOF"
    set -euo pipefail


    "$FLOX_BIN" activate -d two --start-services -- bash script_inner.sh

    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed
EOF
  )
}

# ---------------------------------------------------------------------------- #

@test "remote: only have a single instance of services" {
  setup_sleeping_services
  floxhub_setup "flox"
  "$FLOX_BIN" push --owner "$OWNER"
  assert_success

  mkfifo started finished
  "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- bash <(cat <<'EOF'
    echo > started
    timeout 2 cat finished
EOF
  ) &
  timeout 2 cat started

  run "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- bash -c \
    'echo > finished'
  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"
}

@test "remote: can interact with services from outside the activation" {
  setup_sleeping_services
  floxhub_setup "flox"
  "$FLOX_BIN" push --owner "$OWNER"
  assert_success

  mkfifo started finished
  "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- bash <(cat <<'EOF'
    echo > started
    timeout 2 cat finished
EOF
  ) &
  timeout 2 cat started

  run "$FLOX_BIN" services status -r "${OWNER}/${PROJECT_NAME}"
  assert_success
  assert_output --regexp "one +Running"
  assert_output --regexp "two +Running"
  echo > finished
}

# ---------------------------------------------------------------------------- #


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
  assert_output --partial "Service 'invalid' does not exist."
  assert_output --partial "Services not started or quit unexpectedly."
}

@test "start: errors if service not available" {

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
    invalid.command = "sleep infinity"
    invalid.systems = ["dummy-system"]
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    # don't set -euo pipefail because we expect these to fail
    "$FLOX_BIN" services start invalid
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_failure
  assert_output --partial "Service 'invalid' is not available on '$NIX_SYSTEM'."
  assert_output --partial "Services not started or quit unexpectedly."
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

@test "start: only starts supported services" {


  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = "sleep infinity"
    two.command = "sleep infinity"
    two.systems = ["dummy-system"]
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    # don't set -euo pipefail because we expect these to fail

    "$FLOX_BIN" -vvv services start
    "$FLOX_BIN" -vvvv services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT" 3>&-
  assert_success
  assert_output --partial "Service 'one' started."
  assert_output --partial "one        Running"
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
  assert_output --partial "Service 'two' was defined after services were started."
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
    # The timeout below has timed out,
    # but it's hard to reproduce,
    # so add set -x for more info in the future.
    set -euxo pipefail

    timeout 2 bash -c "set -x; while ! overmind status; do sleep .1; done"

    "$FLOX_BIN" services status
    "$FLOX_BIN" services stop
EOF
  )"

  "$FLOX_BIN" activate -s -- bash -c "$SCRIPT"
  timeout 2 bash -c "while [ -e "$PWD/overmind.sock" ]; do sleep .1; done"
}

@test "activate: picks up changes after environment modification when all services have stopped" {

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
  mkfifo fifo
  "$FLOX_BIN" activate -s -- echo \> fifo &
  activate_pid="$!"

  # Since `one` is just an `echo` it will complete almost immediately once it has
  # started, we just need to make we wait until after it has started.
  echo "waiting for initial service to complete" >&3
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed

  run "$FLOX_BIN" services logs one
  assert_success
  assert_output "foo_one"

  MANIFEST_CONTENTS_2="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo $FOO"
    two.command = "sleep infinity"

    [hook]
    on-activate = "export FOO=foo_two"
EOF
  )"

  echo "$MANIFEST_CONTENTS_2" | "$FLOX_BIN" edit -f -

  "$FLOX_BIN" activate -s -- true

  # Make sure we avoid a race of service one failing to complete
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed

  # The added service should be running.
  "${TESTS_DIR}"/services/wait_for_service_status.sh two:Running

  # The modified value of FOO should be printed.
  run "$FLOX_BIN" services logs one
  assert_success
  assert_output "foo_two"

  # Technically this should be a teardown step
  # The test will hang forever if it fails and doesn't get here
  timeout 2 cat fifo
}
