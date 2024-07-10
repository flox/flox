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
  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    echo "looking for file"
    [ -e hello.txt ]
    echo "found it"
EOF
)
  assert_success
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

# ---------------------------------------------------------------------------- #

# bats test_tags=services,services:stop
@test "stop: can't be used without feature flag" {
  run "$FLOX_BIN" services stop
  assert_failure
  assert_output "❌ ERROR: services are not enabled"
}

# bats test_tags=services,services:stop
@test "stop: can't be used outside an activation that has services" {
  export FLOX_FEATURES_SERVICES=true
  run "$FLOX_BIN" services stop
  assert_failure
  assert_output "❌ ERROR: services have not been started in this activation"
}

# bats test_tags=services,services:stop
@test "stop: errors if a service doesn't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    # TODO: Replace process-compose stop call.
    # "$FLOX_BIN" services stop invalid
    "$PROCESS_COMPOSE_BIN" process stop invalid
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_failure
  assert_output --partial "invalid is not running"
}

# bats test_tags=services,services:stop
@test "stop: errors if one of multiple services don't exist" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    # TODO: Replace process-compose stop call.
    # "$FLOX_BIN" services stop one invalid
    "$PROCESS_COMPOSE_BIN" process stop one invalid
EOF
)
  assert_failure
  assert_output --partial "invalid is not running"
}

# bats test_tags=services,services:stop
@test "stop: errors if service socket isn't responding" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    export _FLOX_SERVICES_SOCKET=invalid
    # TODO: Replace process-compose stop call.
    # "$FLOX_BIN" services stop one invalid
    export PC_SOCKET_PATH="${_FLOX_SERVICES_SOCKET}"
    "$PROCESS_COMPOSE_BIN" process stop one
EOF
)
  assert_failure
  assert_output --partial "connect: no such file or directory"
}

# bats test_tags=services,services:stop
@test "stop: stops all services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    "$FLOX_BIN" services stop
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Completed +"
}

# bats test_tags=services,services:stop
@test "stop: stops a single service" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    "$FLOX_BIN" services stop one
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Running +"
}

# bats test_tags=services,services:stop
@test "stop: stops multiple services" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    "$FLOX_BIN" services stop one two
    "$PROCESS_COMPOSE_BIN" process list --output wide
EOF
)
  assert_success
  assert_output --regexp " +one +default +Completed +"
  assert_output --regexp " +two +default +Completed +"
}

# bats test_tags=services,services:stop
@test "stop: errors if service is already stopped" {
  export FLOX_FEATURES_SERVICES=true
  setup_sleeping_services

  run "$FLOX_BIN" activate -- bash <(cat <<'EOF'
    source "${TESTS_DIR}/services/start_and_cleanup.sh"
    # TODO: Replace process-compose stop call.
    # "$FLOX_BIN" services stop one
    # "$FLOX_BIN" services stop one
    "$PROCESS_COMPOSE_BIN" process stop one
    "$PROCESS_COMPOSE_BIN" process stop one
EOF
)
  # TODO: assert_success
  assert_failure
  assert_output --partial "one is not running"
}
