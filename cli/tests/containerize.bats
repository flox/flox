#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox conteinerize
#
# bats file_tags=containerize
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

env_setup_catalog() {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  "$FLOX_BIN" edit -f "$TESTS_DIR/container/manifest1.toml"
}

# podman writes containers to ~/.local/share/containers/storage
# using an overlayfs.
# However, that directory is not writable
# and thus fails to be deleted by bats as part of the test teardown.
podman_cache_reset() {
  # echo "Resetting podman cache" >&3
  is_linux && podman system reset --force
  true
}

# We need to handle some directories globally for this file, so we need a
# different setup routine than the typical `home_setup` function.
podman_home_setup() {
  if [[ "${__FT_RAN_HOME_SETUP:-}" = "real" ]]; then
    export FLOX_TEST_HOME="$REAL_HOME"
    export HOME="$REAL_HOME"
  else
    tmpdir="$(mktemp -d "/tmp/home.XXXXXX")"
    mkdir -p "$tmpdir"
    export FLOX_TEST_HOME="$tmpdir"
    # Force recreation on `home' on every invocation.
    unset __FT_RAN_HOME_SETUP
  fi
  echo "Podman home dir: $FLOX_TEST_HOME" >&3
  export __FT_RAN_HOME_SETUP="$FLOX_TEST_HOME"
}

podman_xdg_vars_setup() {
  home_dir="$1"; shift;
  short_tmp_dir="$1"; shift;

  xdg_reals_setup
  # These get unset by the preceding function call and must be restored in order
  # to use a single podman machine across the test run
  export XDG_CONFIG_HOME="$short_tmp_dir/.config"
  export XDG_DATA_HOME="$short_tmp_dir/.local/share"
  export XDG_RUNTIME_DIR="$short_tmp_dir/run"

  test_cache_dir="${home_dir:?}/.cache"
  test_state_dir="${home_dir:?}/.local/state"

  # Create all of the directories
  mkdir -p "$home_dir"
  mkdir -p "$test_cache_dir"
  mkdir -p "$test_state_dir"
  chmod u+w "$home_dir"
  chmod u+w "$test_cache_dir"
  chmod u+w "$test_state_dir"

  # Export the vars
  export XDG_CACHE_HOME="$test_cache_dir"
  export XDG_STATE_HOME="$test_state_dir"
}

# This is the same as the global `flox_vars_setup` except it doesn't run
# `xdg_vars_setup` again.
podman_flox_vars_setup() {
  # We store sockets in FLOX_CACHE_DIR,
  # so create cache in /tmp since TMPDIR may result in too long of a path.
  FLOX_CACHE_DIR="$(mktemp -d /tmp/flox.tests.XXXXXX)"
  export FLOX_CACHE_DIR
  export FLOX_CONFIG_DIR="$XDG_CONFIG_HOME/flox"
  export FLOX_DATA_HOME="$XDG_DATA_HOME/flox"
  export FLOX_STATE_HOME="$XDG_STATE_HOME/flox"
  export FLOX_META="$FLOX_CACHE_DIR/meta"
  export FLOX_ENVIRONMENTS="$FLOX_DATA_HOME/environments"
  export HOME="${FLOX_TEST_HOME:-$HOME}"
}


podman_dirs_setup() {
  # Podman creates deeply nested directories and stores sockets in some of them,
  # so we need to create locations to store those with shorter paths than what
  # we'd get nesting them under `/tmp/nix-shell.XXXXXX/bats-run-XXXXXX`.
  export SHORT_TMP="$(mktemp -d "/tmp/XXXXXX")"
  export TMPDIR="$SHORT_TMP"
  export XDG_CONFIG_HOME="$SHORT_TMP/.config"
  export XDG_DATA_HOME="$SHORT_TMP/.local/share"
  export XDG_RUNTIME_DIR="$SHORT_TMP/run"
  echo "Podman XDG root: $SHORT_TMP" >&3
  mkdir -p "$XDG_CONFIG_HOME/containers"
  mkdir -p "$XDG_DATA_HOME"
  mkdir -p "$XDG_RUNTIME_DIR"

  # Don't require Rosetta unless cross-compiling on builders.
  if [[ "${NIX_SYSTEM}" != "x86_64-darwin" ]]; then
    cat > "$XDG_CONFIG_HOME/containers/containers.conf" <<'EOF'
[machine]
rosetta = false
EOF
  fi
  
  podman_home_setup
  podman_xdg_vars_setup "$FLOX_TEST_HOME" "$SHORT_TMP"
  podman_flox_vars_setup
}

start_podman_machine() {
  machine="$(podman machine list -n)"
  if [ -z "$machine" ]; then
    echo "Creating podman machine" >&3
    podman machine init -v /tmp:/tmp -v /Users:/Users -v /private:/private
  fi
  echo "Starting podman machine" >&3
  podman machine start
}

# ---------------------------------------------------------------------------- #

setup() {
  setup_isolated_flox
  project_setup

  # flox does not allow to set a $HOME
  # that does not correspond to the effective user's,
  # but podman requires the policy.json set in the **test user's** $HOME,
  # or otherwise fails with
  #
  # Error: payload does not match any of the supported image formats:
  #  * oci: open /etc/containers/policy.json: no such file or directory
  #  * oci-archive: open /etc/containers/policy.json: no such file or directory
  #  * docker-archive: open /etc/containers/policy.json: no such file or directory
  #  * dir: open /etc/containers/policy.json: no such file or directory
  #
  # (The fact that podman _also_ looks in HOME/.config/containers/policy.json,
  #  but refuses to mention it in the error message, notwithstanding.)
  #
  # To work around this wrap podman in a script that sets the HOME to the test user's.
  # 一点傻傻地，但是有效。
  mkdir -p "$BATS_TEST_TMPDIR/bin"
  ORIGINAL_PODMAN="$(command -v podman)"
  cat > "$BATS_TEST_TMPDIR/bin/podman" << EOF
#!/usr/bin/env bash
HOME=$HOME USER=podman-user exec $ORIGINAL_PODMAN "\$@"
EOF

  chmod +x "$BATS_TEST_TMPDIR/bin/podman"
  export PATH="$BATS_TEST_TMPDIR/bin:$PATH"
}

setup_file() {
  common_file_setup
  # There seems to be a deadlock when running tests in parallel
  # either due to podman, or deleting the podman cache.
  # Since this started with the addition of tests
  # for loading containers into podman from flox,
  # fd3 issues are possible as well.
  # For the sake of getting the tests to pass, we'll disable parallelism.
  # this slows down the tests, but since they already run in parallel
  # with other groups this won't slow down the overall test suite.
  # As a side effect the individual tests will run faster
  # because podman does not need to serialize writes to the cache.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true

  # Only for macOS, don't force rootless on Linux.
  if ! is_linux; then
    podman_dirs_setup
    start_podman_machine
  fi

  mkdir -p "$XDG_CONFIG_HOME/containers"
  echo '{ "default": [ {"type": "insecureAcceptAnything"} ] }' > "$XDG_CONFIG_HOME/containers/policy.json"
}

teardown() {
  project_teardown
  common_test_teardown
}

teardown_file() {
  podman_cache_reset
  common_file_teardown

  if ! is_linux; then
    podman machine stop
    rm -rf "$SHORT_TMP"
    rm -rf "$FLOX_TEST_HOME"
  fi
}

# ---------------------------------------------------------------------------- #

function is_linux() {
  [[ "$(uname)" == "Linux" ]]
}

function skip_if_not_linux() {
  if ! is_linux; then
    skip "Only available on linux"
  fi
}

function skip_if_linux() {
  if is_linux; then
    skip "Not applicable on linux"
  fi
}

# ---------------------------------------------------------------------------- #

# TODO: Implement happy path tests for macOS in
# https://github.com/flox/flox/issues/2466

# bats test_tags=containerize:macos
@test "runtime is required for proxy container on macos" {
  skip_if_linux

  "$FLOX_BIN" init

  run bash -c 'PATH= "$FLOX_BIN" containerize' 3>&-
  assert_failure
  assert_output "❌ ERROR: No container runtime found in PATH.

Exporting a container on macOS requires Docker or Podman to be installed."
}

# bats test_tags=containerize:default-to-file
@test "container is written to a runtime by default" {
  env_setup_catalog

  # Check that podman is installed
  which podman

  run "$FLOX_BIN" containerize
  assert_success
  assert_line --partial "Loaded image: localhost/test:latest"
}

# bats test_tags=containerize:default-to-file
@test "container is written to a file if no runtime is found on PATH on Linux" {
  skip_if_not_linux
  env_setup_catalog

  PATH= run "$FLOX_BIN" containerize
  assert_success
  assert [ -f "test-container.tar" ] # <env-name>-container.tar by default
}

# bats test_tags=containerize:container-tag
@test "container is tagged with specified tag" {
  env_setup_catalog

  # Check that podman is installed
  which podman

  run "$FLOX_BIN" containerize --tag 'sometag'
  assert_success
  assert_line --partial "Loaded image: localhost/test:sometag"
}

# bats test_tags=containerize:piped-to-runtime
@test "container is written to runtime when '--runtime <runtime>' is passed" {
  env_setup_catalog

  run bash -c '"$FLOX_BIN" containerize --tag "runtime" --runtime podman' 3>&-
  assert_success
  assert_line --partial "Loaded image:"

  run --separate-stderr podman run -q -i "localhost/test:runtime" -c 'echo $foo'
  assert_success
}

# bats test_tags=containerize:runtime-not-in-path
@test "error if runtime not in PATH" {
  skip_if_not_linux # macOS checks for the container runtime earlier.
  env_setup_catalog

  run bash -c 'PATH= "$FLOX_BIN" containerize --runtime podman' 3>&-
  assert_failure
  assert_line --partial "Failed to call runtime"
}

function assert_container_output() {
  # check:
  # (1) if the variable `foo = bar` is set in the container
  #   - printed to STDOUT by the container invocation
  # (2) if the binary `hello` is present in the container
  # (3) if the binary `hello` operates as expected
  #   - printed to STDOUT by the on-activate hook, but then
  #     redirected to STDERR by the flox activate script
  assert_equal "${#lines[@]}" 1 # 1 result
  assert_equal "${lines[0]}" "bar"

  # Podman generates some errors/warnings about UIDs/GIDs due to how the rootless
  # setup works: https://github.com/containers/podman/issues/15611
  # Another error you may see is that the container file already exists, which is
  # harmless and can be ignored.
  # So, we can't rely on the *number* of stderr lines, but we know the lines we
  # care about will be the last two lines.

  n_stderr_lines="${#stderr_lines[@]}"
  hello_line="$(($n_stderr_lines - 1))"
  store_path_line="$(($n_stderr_lines - 2))"
  assert_regex "${stderr_lines[$store_path_line]}" "\/nix\/store\/.*\/bin\/hello"
  assert_equal "${stderr_lines[$hello_line]}" "Hello, world!"
}

# bats test_tags=containerize:run-container-i
@test "container can be run with 'podman/docker run' with/without -i'" {
  env_setup_catalog

  # Also tests writing to STDOUT with `-f -`
  CONTAINER_ID="$("$FLOX_BIN" containerize -f - | podman load | sed -nr 's/^Loaded image: (.*)$/\1/p')"

  run --separate-stderr podman run -q -i "$CONTAINER_ID" -c 'echo $foo'
  assert_success
  assert_container_output

  # Next, test without "-i'
  run --separate-stderr podman run "$CONTAINER_ID" -c 'echo $foo'
  assert_success
  assert_container_output
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
