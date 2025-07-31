#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# bats file_tags=build
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  run "$FLOX_BIN" init
  assert_success
  unset output
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/custom/empty/resp.yaml"
}

teardown() {
  project_teardown
  common_test_teardown
}


# ---------------------------------------------------------------------------- #

@test "Build wrapper doesn't modify FLOX_ENV_DIRS" {
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [install]
    # Install hello so we don't have to mock a catalog response for a base page
    hello.pkg-path = "hello"

    [build.print-FLOX_ENV_DIRS]
    command = """
      mkdir -p $out/bin
      cat > "$out/bin/print-FLOX_ENV_DIRS" <<'EOF'
        #!/usr/bin/env bash
        echo "$FLOX_ENV_DIRS"
    EOF

      chmod +x "$out/bin/print-FLOX_ENV_DIRS"
    """
EOF
  )"

  echo "$MANIFEST_CONTENTS" | \
    _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -f -

  "$FLOX_BIN" build

  # A wrapped program should not set FLOX_ENV_DIRS
  run ./result-print-FLOX_ENV_DIRS/bin/print-FLOX_ENV_DIRS
  assert_success
  assert_output ""

  "$FLOX_BIN" init -d consumer
  "$FLOX_BIN" install -d consumer ./result-print-FLOX_ENV_DIRS/bin/print-FLOX_ENV_DIRS

  # The wrapped program should pass through FLOX_ENV_DIRS
  run "$FLOX_BIN" activate -d consumer -- print-FLOX_ENV_DIRS
  assert_success
  assert_output --regexp ".*consumer/.flox/run/$NIX_SYSTEM.consumer.dev"
}

# Check that
# - A built package can find Python modules it was built with
# - A built package can't find Python modules from an environment it's installed
#   to
@test "Build wrapper provides Python modules" {
  cp "$GENERATED_DATA"/envs/build_with_requests/* "$PROJECT_DIR/.flox/env"

  "$FLOX_BIN" build

  run ./result-print-modules/bin/print-modules
  assert_success
  # requests can be found
  assert_output --regexp "\['/nix/store/.*-environment-build-print-modules/lib/python3.13/site-packages/requests'\]"
  # Confirm toml cannot be found
  assert_output --partial "Cannot import toml"

  "$FLOX_BIN" init -d consumer
  "$FLOX_BIN" install -d consumer ./result-print-modules/bin/print-modules
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/python-toml.yaml" \
    "$FLOX_BIN" install -d consumer python313Packages.toml

  # Double check toml module can be found with environment activated
  run "$FLOX_BIN" activate -d consumer -- python3 -c "import toml; print(toml.__path__)"
  assert_success
  assert_output --regexp "\['.*consumer/.flox/run/$NIX_SYSTEM.consumer.dev/lib/python3.13/site-packages/toml'\]"

  # Wrapped program can find requests but not toml
  run "$FLOX_BIN" activate -d consumer -- print-modules
  assert_success
  assert_output --regexp "\['/nix/store/.*-environment-build-print-modules/lib/python3.13/site-packages/requests'\]"
  assert_output --partial "Cannot import toml"
}

# ---------------------------------------------------------------------------- #
