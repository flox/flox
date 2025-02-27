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
# TODO: do we also need to do this on macOS?
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
  export __FT_RAN_HOME_SETUP="$FLOX_TEST_HOME"
}

podman_setup() {
  # Populate REAL_XDG_* vars so we can point at the host machine's data/config
  # directories. This is necessary for caching the podman VM across test runs.
  xdg_reals_setup

  # Create a tempdir with a short path to use as a home directory. The path
  # needs to be short because podman and friends create a deeply nested
  # directory structure that can make socket paths that are longer than 108
  # characters.
  podman_home_setup # populates FLOX_TEST_HOME

  # Set TMPDIR to a directory under this home-tempdir otherwise it will be set
  # to a tempdir created for the development shell, and again you might end up
  # with socket paths that are too long.
  export TMPDIR="$FLOX_TEST_HOME/tmp"
  mkdir -p "$TMPDIR"

  # Set the XDG variables to point to the home-tempdir
  export XDG_DATA_HOME="$FLOX_TEST_HOME/.local/share"
  export XDG_STATE_HOME="$FLOX_TEST_HOME/.local/state"
  export XDG_CACHE_HOME="$FLOX_TEST_HOME/.cache"
  export XDG_RUNTIME_DIR="$FLOX_TEST_HOME/run"
  export XDG_CONFIG_HOME="$FLOX_TEST_HOME/.config"

  mkdir -p "$XDG_DATA_HOME"
  mkdir -p "$XDG_STATE_HOME"
  mkdir -p "$XDG_CACHE_HOME"
  mkdir -p "$XDG_RUNTIME_DIR"
  mkdir -p "$XDG_CONFIG_HOME"

  # Set the flox-specific directories to point to this home-tempdir
  export FLOX_CACHE_DIR="$XDG_CACHE_HOME/flox"
  export FLOX_CONFIG_DIR="$XDG_CONFIG_HOME/flox"
  export FLOX_DATA_HOME="$XDG_DATA_HOME/flox"
  export FLOX_STATE_HOME="$XDG_STATE_HOME/flox"
  export FLOX_META="$FLOX_CACHE_DIR/meta"
  export FLOX_ENVIRONMENTS="$FLOX_DATA_HOME/environments"

  # Set HOME
  export HOME="${FLOX_TEST_HOME:?FLOX_TEST_HOME was unset or null}"
}

create_and_start_podman_machine() {
  # For some of these calls you need to not only close FD 3, but also FD 4.
  # I have no idea why.
  echo "Creating podman machine" >&3
  podman machine init -v /tmp:/tmp -v /Users:/Users -v /private:/private flox-containerize-vm 3>&- 4>&-
  echo "Starting podman machine" >&3
  podman machine start flox-containerize-vm 3>&- 4>&-
}

is_local_dev() {
  [ ! -v "FLOX_CI_RUNNER" ]
}

# ---------------------------------------------------------------------------- #

# Identical to `setup_isolated_flox` but doesn't handle FLOX_CACHE_DIR because
# we already handle the socket length issue for Flox by virtue of handling it
# for podman.
podman_setup_isolated_flox() {
  export FLOX_CONFIG_DIR="${BATS_TEST_TMPDIR?}/flox-config"
  export FLOX_DATA_DIR="${BATS_TEST_TMPDIR?}/flox-data"
  export FLOX_STATE_DIR="${BATS_TEST_TMPDIR?}/flox-state"
}

setup() {
  podman_setup_isolated_flox
  project_setup
}

setup_file() {
  echo "FLOX_CI_RUNNER: '${FLOX_CI_RUNNER}'" >&3
  common_file_setup
  # The individual tests run faster this way because podman doesn't need to
  # serialize writes to the cache, and subsequent runs can reuse the already
  # built flox container.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true

  # Only for macOS, don't force rootless on Linux.
  if ! is_linux; then
    podman_setup
    if is_local_dev; then
      machine_state="$(XDG_DATA_HOME="$REAL_XDG_DATA_HOME" XDG_CONFIG_HOME="$REAL_XDG_CONFIG_HOME" podman machine inspect flox-containerize-vm | jq -r '.[0].State')"
      if [ "$machine_state" != "running" ]; then
        echo "ERROR: podman VM is not running" >&3
        echo "Start the VM with 'podman machine start flox-containerize-vm'" >&3
        echo "See create_and_start_podman_machine() in containerize.bats to create the VM" >&3
        exit 1
      fi
    else
      create_and_start_podman_machine
    fi
  fi

  mkdir -p "$XDG_CONFIG_HOME/containers"
  echo '{ "default": [ {"type": "insecureAcceptAnything"} ] }' > "$XDG_CONFIG_HOME/containers/policy.json"

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
  ORIGINAL_PODMAN="$(command -v podman)"
  mkdir -p "$FLOX_TEST_HOME/bin"
  if is_local_dev; then
    # Use the host machine's VM for local development.
    # The develop is responsible for starting the VM.
    cat > "$FLOX_TEST_HOME/bin/podman" << EOF
#!/usr/bin/env bash
HOME=$HOME XDG_DATA_HOME="$REAL_XDG_DATA_HOME" XDG_CONFIG_HOME="$REAL_XDG_CONFIG_HOME" exec $ORIGINAL_PODMAN "\$@"
EOF
  else
    cat > "$FLOX_TEST_HOME/bin/podman" << EOF
#!/usr/bin/env bash
HOME=$HOME exec $ORIGINAL_PODMAN "\$@"
EOF
  fi

  chmod +x "$FLOX_TEST_HOME/bin/podman"
  export PATH="$FLOX_TEST_HOME/bin:$PATH:/run/wrappers/bin"

  # Check that podman is functioning
  # and ensure it has created the necessary directories.
  # Without this, starting multiple podman containers in parallel,
  # may cause a race between the containers to create the directories,
  # in particular `$HOME/.ssh`.
  podman ps
}

teardown() {
  project_teardown
  common_test_teardown
}

teardown_file() {
  podman_cache_reset
  if ! is_linux; then
    if ! is_local_dev; then
      podman machine stop flox-containerize-vm
    fi
    rm -rf "$SHORT_TMP"
    rm -rf "$FLOX_TEST_HOME"
  fi
  common_file_teardown
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

  # disable backtrace; we expect this to fail and assert output
  RUST_BACKTRACE=0 run bash -c 'PATH= "$FLOX_BIN" containerize' 3>&-
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
  assert_line "✨ 'test:latest' written to Podman runtime"
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
  assert_line "✨ 'test:sometag' written to Podman runtime"
}

# bats test_tags=containerize:piped-to-runtime
@test "container is written to runtime when '--runtime <runtime>' is passed" {
  env_setup_catalog

  run bash -c '"$FLOX_BIN" containerize --tag "runtime" --runtime podman' 3>&-
  assert_success
  assert_line "✨ 'test:runtime' written to Podman runtime"

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
@test "container can be run with 'podman run' with/without -i'" {
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

@test "config set on image" {
  skip_if_not_linux # config is implemented in the Linux build of flox entirely

  "$FLOX_BIN" init

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [containerize.config]
    user = "user"
    exposed-ports = [ "80/tcp" ]
    cmd = [ "some", "command" ]
    volumes = [ "/some/volume" ]
    working-dir = "/working/dir"
    labels = { "dev.flox.key" = "value" }
    stop-signal = "SIGKILL"
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  TAG="config-set"

  bash -c "$FLOX_BIN containerize --tag $TAG --runtime podman" 3>&- # TODO: why close FD 3?

  run bash -c "podman inspect test:$TAG | jq '.[0].Config | .User, .ExposedPorts, .Cmd, .Volumes, .WorkingDir, .Labels, .StopSignal'"
  assert_success
  assert_output  --partial - <<EOF
"user"
{
  "80/tcp": {}
}
[
  "some",
  "command"
]
{
  "/some/volume": {}
}
"/working/dir"
{
  "dev.flox.key": "value"
}
"SIGKILL"
EOF
}

@test "cmd can run binary from activated environment" {
  "$FLOX_BIN" init

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1
    [install]
    hello.pkg-path = "hello"

    [containerize.config]
    cmd = [ "hello" ]
EOF
  )"

  echo "$MANIFEST_CONTENTS" | _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" edit -f -

  TAG="cmd-runs-in-activation"

  bash -c "$FLOX_BIN containerize --tag $TAG --runtime podman" 3>&- # TODO: why close FD 3?

  run podman run --rm "test:$TAG"
  assert_success
  assert_output --partial "Hello, world!"

  # Verify that the `activate` entrypoint is still used when an ad-hoc command
  # is used and that (since it's quicker than executing a separate test)
  # `FLOX_ENV_*` are set correctly.
  run podman run --rm "test:$TAG" -c 'echo $FLOX_ENV_CACHE'
  assert_success
  assert_output "/tmp"

  run podman run --rm "test:$TAG" -c 'echo $FLOX_ENV_DESCRIPTION'
  assert_success
  assert_output "test"
}

@test "container with user:group set can run as specified user:group" {
  skip_if_not_linux # config is implemented in the Linux build of flox entirely

  "$FLOX_BIN" init

  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [containerize.config]
    user = "foo:bar"
EOF
  )"

  echo "$MANIFEST_CONTENTS" | _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" "$FLOX_BIN" edit -f -

  TAG="whoami-in-container"

  bash -c "$FLOX_BIN containerize --tag $TAG --runtime podman" 3>&- # TODO: why close FD 3?

  run podman run --rm "test:$TAG" 'whoami'
  assert_success
  assert_output --partial "foo"

  run bash -c "podman inspect test:$TAG | jq '.[0].Config | .User'"
  assert_success
  assert_output  --partial - <<EOF
"foo:bar"
EOF
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
