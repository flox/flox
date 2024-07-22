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
  assert_output --partial "v1.6.1"
}

@test "process-compose can run generated config file" {
  export FLOX_FEATURES_SERVICES=true
  "$FLOX_BIN" init
  run "$FLOX_BIN" edit -f "${TESTS_DIR}/services/touch_file.toml"
  assert_success
  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
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
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    redis-cli -p "${REDIS_PORT}" ping
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
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop invalid
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: service 'invalid' is not running"
}

# bats test_tags=services:stop
@test "stop: errors after stopping one service if subsequent service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop one invalid || exit_code=$?
    "$PROCESS_COMPOSE_BIN" process list --output wide
    exit $exit_code
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: service 'invalid' is not running"
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Running +"
}

# bats test_tags=services:stop
@test "stop: errors without stopping any services if preceeding service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    exit_code=0
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop invalid one || exit_code=$?
    "$PROCESS_COMPOSE_BIN" process list --output wide
    exit $exit_code
EOF
)
  assert_failure
  assert_output --partial "❌ ERROR: service 'invalid' is not running"
  assert_output --regexp " +one +default +Running +"
  assert_output --regexp " +two +default +Running +"
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
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --partial "✅ Service 'two' stopped"
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Completed +"
}

# bats test_tags=services:stop
@test "stop: stops a single service" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop one
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Running +"
}

# bats test_tags=services:stop
@test "stop: stops multiple services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop one two
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --partial "✅ Service 'one' stopped"
  assert_output --partial "✅ Service 'two' stopped"
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Completed +"
}

# bats test_tags=services:stop
@test "stop: errors if service is already stopped" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate --start-services -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/wait_and_cleanup.sh"
    "$FLOX_BIN" services stop one
    "$PROCESS_COMPOSE_BIN" process list --output wide
    "$FLOX_BIN" services stop one
EOF
)
  assert_failure
  assert_output --regexp " +one +default +Completed +"
  assert_output --partial "❌ ERROR: service 'one' is not running"
}
