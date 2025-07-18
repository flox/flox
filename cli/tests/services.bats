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

# Wait, with a poll and timeout, for partial contents to appear in a file
wait_for_partial_file_content() {
  file="${1?}"
  expected="${2?}"

  export file expected
  timeout 1s bash -c '
    while ! grep -q "$expected" "$file"; do
      sleep 0.1s
    done
  '
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  cat_teardown_fifo
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
  wait_for_watchdogs "$PROJECT_DIR" || return 1
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
  assert_output --partial "v1.64.1"
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

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/redis.yaml" \
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

@test "help for the command is displayed with no args" {

    RUST_LOG=debug run "$FLOX_BIN" services
    assert_success
    assert_output --partial "Interact with services"
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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"

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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
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
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_hello.yaml" \
    run "$FLOX_BIN" install hello

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
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

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
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

  commands=("logs" "stop")
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
    wait_for_watchdogs "$PROJECT_DIR"
  done
}

# NB: There is a corresponding test in `activate.bats`.
@test "start, restart: refuses to attach to an older activations.json version" {
  setup_sleeping_services

  # Prevent backtraces from `flox-activations` leaking into output.
  unset RUST_BACKTRACE

  export -f jq_edit
  commands=("start" "restart")
  for command in "${commands[@]}"; do
    echo "Testing: flox services $command"
    command="$command" run "$FLOX_BIN" activate -- bash <(
      cat << 'EOF'
        echo "$PPID" > activation_pid

        ACTIVATIONS_DIR=$(dirname "$_FLOX_ACTIVATION_STATE_DIR")
        ACTIVATIONS_JSON="${ACTIVATIONS_DIR}/activations.json"
        jq_edit "$ACTIVATIONS_JSON" '.version = 0'

        "$FLOX_BIN" services "$command"
EOF
    )

    # Capture from the previous activation.
    ACTIVATION_PID=$(cat activation_pid)

    assert_failure
    assert_output "❌ ERROR: failed to run activation script: Error: This environment has already been activated with an incompatible version of 'flox'.

Exit all activations of the environment and try again.
PIDs of the running activations: ${ACTIVATION_PID}"

    # give the watchdog a chance to clean up the services before the next iteration
    wait_for_watchdogs "$PROJECT_DIR"
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
  run --separate-stderr "$FLOX_BIN" activate --start-services -- bash <(
    cat << 'EOF'

    # ensure some logs are printed for both services then stop the log reader
    # both processes write to the pipe once to signal they have written _something_
    # (they will also _wait_ until the pipe is read)
    read < ./resume-one.pipe
    read < ./resume-mostly-deterministic.pipe

    "$FLOX_BIN" services logs --follow > logs &
    logs_pid="$!"

    timeout 5s bash -c '
      while ! grep "^mostly-deterministic: " logs || ! grep "^one                 : " logs; do
        sleep .1
      done
      exit 0
    '
    status="$?"

    [ $status = 0 ] || echo "didn't find expected logs"

    # kill log reading, because with `--follow` the process will block indefinitely
    kill -SIGTERM "$logs_pid"

    exit $status
EOF
  )

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

    timeout 5s bash -c '
      while ! grep "^mostly-deterministic: " logs || ! grep "^one                 : " logs; do
        sleep .1
      done
      exit 0
    '
    status="$?"
    [ $status = 124 ] || echo "didn't find expected logs"


    # kill log reading, because with `--follow` the process will block indefinitely
    kill -SIGTERM "$logs_pid"


    exit $status
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

@test "status: prints requested services (before start)" {
  setup_sleeping_services

  run "$FLOX_BIN" services status one
  assert_success


  # Note that the PID is omitted if the service hasn't been started
  assert_output --regexp "NAME +STATUS +PID"
  assert_output --regexp "one +Stopped +"
}

@test "status: prints requested services (after start)" {
  setup_sleeping_services

  run "$FLOX_BIN" activate -s -- bash -c '
    "$FLOX_BIN" services status one
  '
  assert_success

  assert_output --regexp "NAME +STATUS +PID"
  assert_output --regexp "one +Running +[0-9]+"
}

# ---------------------------------------------------------------------------- #

@test "activate services: shows warning when services already running" {
  setup_sleeping_services

  mkfifo activate_started_fifo
  TEARDOWN_FIFO="$PROJECT_DIR/finished"
  mkfifo "$TEARDOWN_FIFO"
  "$FLOX_BIN" activate -s -- bash -c "echo > activate_started_fifo && echo > $TEARDOWN_FIFO" &

  # Make sure the first `process-compose` gets up and running
  cat activate_started_fifo
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Running

  run "$FLOX_BIN" activate -s -- true
  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"
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
    while ! grep 'flox_watchdog: starting' \"$PROJECT_DIR\"/.flox/log/watchdog.*.log*; do
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

@test "activate: starts services for in-place activations" {
  setup_sleeping_services

  # Run in a sub-shell so that `wait_for_watchdogs` in `teardown` can verify
  # that the activation is cleaned up on exit and implicitly that services are
  # shutdown.
  run bash -c '
    set -euo pipefail
    eval "$("$FLOX_BIN" activate --start-services)"
    "$FLOX_BIN" services status
  '

  assert_success
  assert_output --regexp "one +Running"
  assert_output --regexp "two +Running"
}

# ---------------------------------------------------------------------------- #

@test "remote: only have a single instance of services" {
  setup_sleeping_services
  floxhub_setup "flox"
  "$FLOX_BIN" push --owner "$OWNER"
  assert_success
  TEARDOWN_FIFO="$PROJECT_DIR/finished"

  ensure_remote_environment_built "$OWNER/$PROJECT_NAME"
  mkfifo started "$TEARDOWN_FIFO"
  _FLOX_TESTING_NO_BUILD=true "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- bash <(cat <<'EOF'
    echo > started
    echo > finished
EOF
  ) &
  timeout 8 cat started

  run "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- true
  assert_success
  assert_output --partial "⚠️  Skipped starting services, services are already running"
}

@test "remote: can interact with services from outside the activation" {
  setup_sleeping_services
  floxhub_setup "flox"
  "$FLOX_BIN" push --owner "$OWNER"
  assert_success

  ensure_remote_environment_built "$OWNER/$PROJECT_NAME"
  mkfifo started finished
  _FLOX_TESTING_NO_BUILD=true "$FLOX_BIN" activate --start-services -r "${OWNER}/${PROJECT_NAME}" -- bash <(cat <<'EOF'
    echo > started
    timeout 8 cat finished
EOF
  ) &
  timeout 8 cat started

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
    "$FLOX_BIN" services start one invalid || true
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success # only a success because we ignore the error from start
  assert_output --partial "Service 'invalid' does not exist."
  assert_output --partial "one        Stopped"
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
    "$FLOX_BIN" services start invalid || true
    "$FLOX_BIN" services status
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success # only a success because we ignore the error from start
  assert_output --partial "Service 'invalid' is not available on '$NIX_SYSTEM'."
  assert_output --partial "invalid    Stopped"
  assert_output --partial "one        Stopped"
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
    "$FLOX_BIN" -vvv services start || true
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
    daemonize.pkg-path = "daemonize"

    [services.daemonized_sleep]
    command = '''
      daemonize -p "$FLOX_ENV_PROJECT/pidfile" "$(which sleep)" 999999
    '''
    is-daemon = true
    shutdown.command = '''
      kill -9 "$(cat $(pwd)/pidfile)"
    '''
EOF
)"

  "$FLOX_BIN" init
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/daemonize.yaml"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  run "$FLOX_BIN" activate -s -- bash "${TESTS_DIR}/services/check_daemon_process.sh"
  assert_success
}

@test "activate: picks up changes after environment modification when all services have stopped" {

  MANIFEST_CONTENTS_1="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo $FOO > out"

    [hook]
    on-activate = "export FOO=foo_one"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS_1" | "$FLOX_BIN" edit -f -

  # Start a background activation with an initial value of FOO.
  TEARDOWN_FIFO="$PROJECT_DIR/finished"
  mkfifo "$TEARDOWN_FIFO"
  "$FLOX_BIN" activate -s -- echo \> "$TEARDOWN_FIFO" &

  # The initial value of FOO should be written.
  wait_for_file_content out foo_one

  MANIFEST_CONTENTS_2="$(cat << "EOF"
    version = 1

    [services]
    one.command = "echo $FOO > out"
    two.command = "sleep infinity"

    [hook]
    on-activate = "export FOO=foo_two"
EOF
  )"

  echo "$MANIFEST_CONTENTS_2" | "$FLOX_BIN" edit -f -

  # Start a new and concurrent activation and services with a modified value of FOO.
  "$FLOX_BIN" activate -s -- true

  # The modified value of FOO should be written.
  wait_for_file_content out foo_two

  # The added service should be running.
  "${TESTS_DIR}"/services/wait_for_service_status.sh two:Running

}

@test "services stop after multiple activations of an environment exit" {
  setup_sleeping_services

  # Start a first activation
  mkfifo started_1
  # Will get cat'ed in teardown
  TEARDOWN_FIFO="$PROJECT_DIR/finished_1"
  mkfifo "$TEARDOWN_FIFO"
  "$FLOX_BIN" activate --start-services -- bash -c "echo > started_1 && echo > $TEARDOWN_FIFO" &
  timeout 2 cat started_1

  # Check that services and watchdog are both running
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Running
  watchdog_1_log="$(echo $PROJECT_DIR/.flox/log/watchdog.*.log.*)"
  run cat "$watchdog_1_log"
  assert_success
  assert_output --partial "woof"

  # Start a second activation
  MANIFEST_APPEND="$(cat << "EOF"
[vars]
dummy = "whatever"
EOF
  )"
  NEW_MANIFEST_CONTENTS="$("$FLOX_BIN" list -c | cat - <(echo "$MANIFEST_APPEND"))"
  echo "$NEW_MANIFEST_CONTENTS"
  export NEW_MANIFEST_CONTENTS
  run bash -c 'echo "$NEW_MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -'
  assert_success
  assert_output --partial "Environment successfully updated."

  mkfifo started_2
  mkfifo finished_2

  "$FLOX_BIN" activate --start-services -- bash -c "echo > started_2 && echo > finished_2" 2>output &

  timeout 2 cat started_2
  # Swap out teardown fifo and immediately teardown first activation
  # Wait for 2nd activation to start before tearing down the 1st
  # otherwise services might get stopped
  TEARDOWN_FIFO="$PROJECT_DIR/finished_2"
  timeout 2 cat finished_1
  run cat output
  assert_output --partial "⚠️  Skipped starting services, services are already running"

  # Check that watchdog 1 has finished cleanup
  run cat "$watchdog_1_log"
  assert_output --partial "woof"
  wait_for_partial_file_content "$watchdog_1_log" "finished cleanup"
  rm "$watchdog_1_log"

  # Check that watchdog 2 is running
  watchdog_2_log="$(echo $PROJECT_DIR/.flox/log/watchdog.*.log.*)"
  run cat "$watchdog_2_log"
  assert_output --partial "woof"
  refute_output "finished cleanup"

  # Even though watchdog 1 cleaned up, services should still be running
  "${TESTS_DIR}"/services/wait_for_service_status.sh one:Running

  # Teardown 2nd activation and wait for watchdog to cleanup
  cat finished_2
  unset TEARDOWN_FIFO
  wait_for_partial_file_content "$watchdog_2_log" "finished cleanup"

  # Make sure services have stopped
  timeout 1s bash -c '
    "${TESTS_DIR}"/services/wait_for_service_status.sh one:Stopped two:Stopped
  '
}

@test "vars: service-level variables are set" {

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [services]
    one.command = '''
      echo "hello $myvar"
    '''
    one.vars.myvar = "some_value"
EOF
  )"

  "$FLOX_BIN" init
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  SCRIPT="$(cat << "EOF"
    set -euo pipefail

    "$FLOX_BIN" services start one
    "$FLOX_BIN" services logs -n 10 one
EOF
  )"

  run "$FLOX_BIN" activate -- bash -c "$SCRIPT"
  assert_success
  assert_output --partial "Service 'one' started."
  assert_output --partial "some_value"
}
