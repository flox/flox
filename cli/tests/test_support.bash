#! /usr/bin/env bash
# ============================================================================ #
#
# Helper utilities shared in common by most tests - particularly
# the routines `setup_*' and `teardown_*'.
#
# By loading this file you will get the common routines as your default; but
# these can be redefined in a particular test file at any point after loading
# and before writing test definitions.
#
# ---------------------------------------------------------------------------- #
#
# NOTE: This file is processed after `setup_suite.bash'.
#
# ---------------------------------------------------------------------------- #

load setup_suite.bash

# ---------------------------------------------------------------------------- #

require_expect() {
  if ! command -v expect > /dev/null 2>&1; then
    echo "ERROR: expect library needs to be in PATH."
    return 1
  fi
}

# ---------------------------------------------------------------------------- #

# `/foo/bar/flox/tests/foo.bats' -> `foo'
setup_test_basename() {
  BATS_TEST_BASENAME="${BATS_TEST_FILENAME##*/}"
  export BATS_TEST_BASENAME="${BATS_TEST_BASENAME%.bats}"
}

# ---------------------------------------------------------------------------- #

# Generate an env name base on the test file's name, setting `TEST_ENVIRONMENT'.
#
# Ex: `test/foo.bats'  ->  `_testing_foo'
setup_file_envname() {
  setup_test_basename
  # Append random number to test environment to avoid collisions when
  # pushing/pulling to floxhub.
  local _random_8digits=$(shuf -i 10000000-99999999 -n 1)
  : "${TEST_ENVIRONMENT:=${FLOX_TEST_ENVNAME_PREFIX}${BATS_TEST_BASENAME}-$_random_8digits}"
  export TEST_ENVIRONMENT
}

# ---------------------------------------------------------------------------- #

# Generate an env name base on the test file's name and the current test number,
# setting `TEST_ENVIRONMENT'.
#
# Ex: `test/foo.bats:@test#4'  ->  `_testing_foo_4'
setup_test_envname() {
  setup_test_basename
  setup_file_envname
  TEST_ENVIRONMENT="$TEST_ENVIRONMENT-$BATS_TEST_NUMBER"
  export TEST_ENVIRONMENT
}

# ---------------------------------------------------------------------------- #

# Build `hello' and root it temporarily so it can be used as an
# install target in various tests.
# This symlink is deleteed by `common_teardown'.
hello_pkg_setup() {
  if [[ -n "${__FT_RAN_HELLO_PKG_SETUP:-}" ]]; then return 0; fi
  export HELLO_LINK="$BATS_SUITE_TMPDIR/gc-roots/hello"
  mkdir -p "${HELLO_LINK%/*}"
  $NIX_BIN --experimental-features "nix-command flakes" build 'nixpkgs#hello' --out-link "$HELLO_LINK"
  HELLO_PACKAGE="$(readlink -f "$HELLO_LINK")"
  # Get first 8 characters of store path hash.
  HELLO_PACKAGE_FIRST8="${HELLO_PACKAGE#"${NIX_STORE:-/nix/store}/"}"
  HELLO_PACKAGE_FIRST8="${HELLO_PACKAGE_FIRST8:0:8}"
  export HELLO_PACKAGE HELLO_PACKAGE_FIRST8
  export __FT_RAN_HELLO_PKG_SETUP=:
}

# ---------------------------------------------------------------------------- #

# floxhub_setup <owner>
#
# * sets up a local "floxhub" repo for the given owner.
#   can be called multiple times with different owners.
# * sets `FLOX_FLOXHUB_TOKEN` to a dummy value so flox _thinks_ its logged in.
#   the token is a valid JWT token with a dummy payload:
#
#     { "https://flox.dev/handler": "test", "exp": 9999999999 }
#
# This is used by tests that need to push/pull to/from floxhub.
# In the future we may want to use a local floxhub server instead.
floxhub_setup() {
  OWNER="$1"
  shift
  export FLOX_FLOXHUB_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjk5OTk5OTk5OTl9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74"
  export FLOX_FLOXHUB_PATH="$BATS_TEST_TMPDIR/floxhub"
  export FLOXHUB_FLOXMETA_DIR="$FLOX_FLOXHUB_PATH/$OWNER/floxmeta"

  mkdir -p "$FLOX_FLOXHUB_PATH"
  mkdir -p "$FLOXHUB_FLOXMETA_DIR"
  git -C "$FLOXHUB_FLOXMETA_DIR" init --bare
  git -C "$FLOXHUB_FLOXMETA_DIR" config user.name "test"
  git -C "$FLOXHUB_FLOXMETA_DIR" config user.email "test@email.address"

  export _FLOX_FLOXHUB_GIT_URL="file://$FLOX_FLOXHUB_PATH"
}

# Isolate flox config, data, and cache from the potentially shared
# xdg directories.
# This is necessary as other wisemultiple tests contest for the same
# resources, e.g.:
# * the global manifest and lockfile
#   + created by multiple processes
#   + deleted by some but assumed present by others
#   + updated, upgraded, reset
# * floxmeta clones for managed environments
#   + same _owner_ and project name being reused
#   + environments are created/deleted/edited concurrently
#     -> git errors, and just plain data corruption
# * local ephemeral environments by `--remote` commands.
#   + git concurrency
#
# nix caches and pkgdb caches remain shared, since they are effectively read-only.
setup_isolated_flox() {
  export FLOX_CONFIG_DIR="${BATS_TEST_TMPDIR?}/flox-config"
  export FLOX_DATA_DIR="${BATS_TEST_TMPDIR?}/flox-data"
  # Don't use BATS_TEST_TMPDIR since we store sockets in FLOX_CACHE_DIR,
  # and BATS_TEST_TMPDIR will likely be too long.
  # Create within the existing FLOX_CACHE_DIR so this gets cleaned up by
  # `common_suite_teardown`.
  FLOX_CACHE_DIR="$(mktemp -d -p "$FLOX_CACHE_DIR")"
  export FLOX_CACHE_DIR
}


# ---------------------------------------------------------------------------- #

# common_file_setup [HOME_STYLE ::= (suite|file|test)]
# ----------------------------------------------------
# Run once for a given `bats' test file.
# This function may be redefined by individual test files, but running
# `common_file_setup' is the recommended minimum.
#shellcheck disable=SC2120
common_file_setup() {
  # Generate a `TEST_ENVIRONMENT' name.
  setup_file_envname
  # Remove any vestiges of previous test runs.
  deleteEnvForce "$TEST_ENVIRONMENT"
  # Setup a homedir associated with this file.
  if [[ "${1:-suite}" != test ]]; then home_setup "${1:-suite}"; fi
}

#shellcheck disable=SC2119
setup_file() { common_file_setup; }

# Added for consistency with `teardown' routines.
common_test_setup() { :; }
setup() { common_test_setup; }

# ---------------------------------------------------------------------------- #

common_file_teardown() {
  # Delete file tmpdir and env unless the user requests to preserve them.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    deleteEnvForce "$TEST_ENVIRONMENT"
    rm -rf "$BATS_FILE_TMPDIR"
    rm -rf "$FLOX_CACHE_DIR"
  fi
  unset FLOX_TEST_HOME
}

teardown_file() { common_file_teardown; }

wait_for_watchdogs() {
  # wait for any running flox-watchdog proceses to finish
  if [[ -n "${FLOX_DATA_DIR:-}" ]]; then
    # This is a hack to essentially do a `pgrep` without having access to `pgrep`.
    # The `ps` prints `<pid> <cmd>`, then we use two separate `grep`s so that the
    # grep command itself doesn't get listed when we search for the data dir.
    # The `sed` removes any leading whitespace,
    # that is present in the output of `ps` on linux aparently?!.
    # The `cut` just extracts the PID.

    local pids
    pids="$(
      ps -Ao pid,args \
      | grep flox-watchdog \
      | grep ${FLOX_DATA_DIR?} \
      | sed 's/^[[:blank:]]*//' \
      | cut -d' ' -f1)"

    # Uncomment to debug which watchdogs are running.
    #
    # echo "FLOX_DATA_DIR => ${FLOX_DATA_DIR?}" >&3
    # ps -Ao pid,args \
    #  | grep flox-watchdog \
    #  >&3

    if [ -n "${pids?}" ]; then
      echo "Waiting for pids: $pids" >&3

      tries=0
      while true; do
        tries=$((tries + 1))
        if ! kill -0 $pids > /dev/null 2>&1; then
          break
        else
          if [[ $tries -gt 1000 ]]; then
            echo "ERROR: flox-watchdog processes did not finish after 10 seconds." >&3
            # This will fail the test giving us a better idea of which watchdog
            # didn't get cleaned up
            exit 1
          fi
          sleep 0.01;
        fi
      done
    fi
  else
    echo "FLOX_DATA_DIR not set, cannot wait for watchdogs." >&3
  fi

}

common_test_teardown() {
  # Delete test tmpdir unless the user requests to preserve them.
  # XXX: We do not attempt to delete envs here.
  if [[ -z "${FLOX_TEST_KEEP_TMP:-}" ]]; then
    rm -rf "$BATS_TEST_TMPDIR" || (ls -laR "$BATS_TEST_TMPDIR"; exit 1);
  fi
}

teardown() { common_test_teardown; }

# Get a system different from the current one.
get_system_other_than_current() {
  # replace linux with darwin or darwin with linux
  case "$NIX_SYSTEM" in
    *-darwin)
      extra_system="${NIX_SYSTEM%%-darwin}-linux"
      ;;
    *-linux)
      extra_system="${NIX_SYSTEM%%-linux}-darwin"
      ;;
    *)
      echo "Unsupported system: $NIX_SYSTEM"
      return 1
  esac
  echo "$extra_system"
}

# Edit a JSON file with `jq' in-place.
jq_edit() {
  local _file="${1?You must provide a target file}"
  local _command="${2?You must provide a jq command}"
  local _tmp
  _tmp="${_file}~"
  jq "$_command" "$_file" > "$_tmp"
  mv "$_tmp" "$_file"
}

dummy_registry() {
  local path="$1"; shift
  local hash="$1"
  REGISTRY_CONTENT="$(cat << EOF
  {
    "version": 1,
    "entries": [
      {
        "hash": "$hash",
        "path": "$path",
        "envs": [
          {
            "created_at": 123,
            "pointer": {
              "name": "dummy_env",
              "version": 1
            }
          }
        ]
      }
    ]
  }
EOF
)"
  echo "$REGISTRY_CONTENT"
}

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
