#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox generations`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=generations

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="project-${BATS_TEST_NUMBER?}"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/${PROJECT_NAME?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

create_environment_with_generations() {
  # Generation 1
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" push --owner owner

  # Generation 2
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # Generation 3
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" install hello
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "owner"

  FLOXHUB_GIT_WARNING=$(cat <<EOF
! Using file://${FLOX_FLOXHUB_PATH} as FloxHub host
'\$_FLOX_FLOXHUB_GIT_URL' is used for testing purposes only,
alternative FloxHub hosts are not yet supported!
EOF
  )
  export FLOXHUB_GIT_WARNING
}
teardown() {
  wait_for_watchdogs "$PROJECT_DIR" || return 1
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

@test "commands are displayed for generations history" {
  create_environment_with_generations

  # Generation 4: switch generation, but set argv[0] to foo
  (exec -a foo "$FLOX_BIN" generations switch 1)

  run "$FLOX_BIN" generations history
  assert_line "Command:    flox push --owner owner"
  assert_line "Command:    flox edit -f -"
  assert_line "Command:    flox install hello"
  # Regardless of argv[0], we always print 'flox'
  assert_line "Command:    flox generations switch 1"
}

@test "activate --generation: works with managed and remote envs" {
  create_environment_with_generations

  # Guard against using 'hello' from the live generation.
  "$FLOX_BIN" generations switch 1

  RUST_BACKTRACE=0 run -127 "$FLOX_BIN" activate --generation 2 -- hello
  assert_failure
  assert_output --partial "hello: command not found"

  run "$FLOX_BIN" activate --generation 3 -- hello
  assert_success
  assert_output - <<EOF
${FLOXHUB_GIT_WARNING?}

Hello, world!
EOF
}

@test "activate --generation: errors for path envs" {
  "$FLOX_BIN" init

  RUST_BACKTRACE=0 run "$FLOX_BIN" activate --generation 3 -- hello
  assert_failure
  assert_output - << EOF
${FLOXHUB_GIT_WARNING?}

✘ ERROR: Generations are only available for environments pushed to floxhub.
The environment ${PROJECT_NAME} is a local only environment.
EOF
}

@test "activate --generation: flox list works with --generation flag" {
  create_environment_with_generations

  # Guard against using 'hello' from the live generation.
  "$FLOX_BIN" generations switch 1

  run "$FLOX_BIN" activate --generation 2 -- "$FLOX_BIN" list --name
  assert_success
  assert_output - <<EOF
${FLOXHUB_GIT_WARNING?}

${FLOXHUB_GIT_WARNING?}

! No packages are installed for your current system ('${NIX_SYSTEM}').

You can see the whole manifest with 'flox list --config'.
EOF

  run "$FLOX_BIN" activate --generation 3 -- "$FLOX_BIN" list --name
  assert_success
  assert_output - <<EOF
${FLOXHUB_GIT_WARNING?}

${FLOXHUB_GIT_WARNING?}

hello
EOF
}

# 'flox services start' performs an "ephemeral" activation, which is more
# cumbersome than 'flox activate -s' and should respect the generation of the
# current activation.
@test "activate --generation: flox services start respects generation" {
  # Generation 1
  "$FLOX_BIN" init --name "test"
  "$FLOX_BIN" push --owner owner

  # Generation 2
  "$FLOX_BIN" edit -f - <<'EOF'
    version = 1

    [services.write_generation]
    command = "echo 'generation 2' > generation"
EOF

  # Generation 3
  "$FLOX_BIN" edit -f - <<'EOF'
    version = 1

    [services.write_generation]
    command = "echo 'generation 3' > generation"
EOF

  SCRIPT="$(cat <<'EOF'
    "$FLOX_BIN" services start
    "${TESTS_DIR}"/services/wait_for_service_status.sh write_generation:Completed
EOF
  )"

  run "$FLOX_BIN" activate --generation 2 -- bash -c "$SCRIPT"
  assert_success
  run cat generation
  assert_success
  assert_output "generation 2"

  run "$FLOX_BIN" activate --generation 3 -- bash -c "$SCRIPT"
  assert_success
  run cat generation
  assert_success
  assert_output "generation 3"
}

test_mutate_with_activate_generation() {
  argv=("$@")
  create_environment_with_generations

  RUST_BACKTRACE=0 run "$FLOX_BIN" activate --generation 3 -- "$FLOX_BIN" "${argv[@]}"
  assert_failure
  assert_output - <<EOF
${FLOXHUB_GIT_WARNING?}

${FLOXHUB_GIT_WARNING?}

✘ ERROR: generations error: Cannot modify environments that are activated with a specific generation.

If you wish to modify the environment at this generation:
- Exit the current activation of the environment
- Activate the environment without specifying a generation
- Optionally switch the live generation: 'flox generation switch 3'
EOF
}

@test "activate --generation: can't mutate with: install" {
  test_mutate_with_activate_generation install foo
}
@test "activate --generation: can't mutate with: uninstall" {
  test_mutate_with_activate_generation uninstall foo
}
@test "activate --generation: can't mutate with: edit" {
  manifest="$(mktemp)"
  echo "version = 1" >> "$manifest"
  test_mutate_with_activate_generation edit -f "$manifest"
  rm "$manifest"
}
@test "activate --generation: can't mutate with: upgrade" {
  test_mutate_with_activate_generation upgrade
}
