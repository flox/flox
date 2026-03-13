#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test minimum-cli-version field
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=minimum-cli-version

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
  common_file_teardown
}

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  # Capture the actual CLI version (major.minor.patch) so tests work even when
  # the Nix wrapper hard-codes FLOX_VERSION (as it does in CI).
  FLOX_CLI_SEMVER="$("$FLOX_BIN" --version | grep -oE '^[0-9]+\.[0-9]+\.[0-9]+')"
}

teardown() {
  project_teardown
}

# ---------------------------------------------------------------------------- #

@test "minimum-cli-version: no warning when satisfied" {
  tomlq --in-place -t '."minimum-cli-version" = "0.0.1"' .flox/env/manifest.toml

  run "$FLOX_BIN" list
  assert_success
  assert_output - << EOF
! No packages are installed for your current system ('${NIX_SYSTEM}').

You can see the whole manifest with 'flox list --config'.
EOF
}

@test "minimum-cli-version: warning when CLI is older" {
  tomlq --in-place -t '."minimum-cli-version" = "99.99.99"' .flox/env/manifest.toml

  run "$FLOX_BIN" list
  assert_success
  assert_output - << EOF
! This environment requires Flox v99.99.99 or later, you have v${FLOX_CLI_SEMVER}.
! No packages are installed for your current system ('${NIX_SYSTEM}').

You can see the whole manifest with 'flox list --config'.
EOF
}

@test "minimum-cli-version: invalid semver is rejected" {
  tomlq --in-place -t '."minimum-cli-version" = "not.a.version"' .flox/env/manifest.toml

  RUST_BACKTRACE=0 run "$FLOX_BIN" list
  assert_failure
  assert_output - << 'EOF'
✘ ERROR: invalid manifest: unexpected character 'n' while parsing major version number
in `minimum-cli-version`
EOF
}
