#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test authentication with `flox_pat_` personal access tokens.
#
# A PAT is opaque: the CLI never parses it, and resolves the identity behind
# it lazily from `GET /accounts/api/v1/accounts/me`. These tests serve `/me` with the
# usual catalog record/replay mechanism (_FLOX_USE_CATALOG_MOCK) using the
# hand-written fixtures in test_data/manually_generated/auth; push/pull
# traffic goes to the usual file-based floxhub.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=auth,auth:pat

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
  common_file_teardown
}

# ---------------------------------------------------------------------------- #

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  floxhub_setup "owner"
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=auth:pat:opaque
@test "pat: an opaque flox_pat_ token is not discarded as invalid" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"

  run "$FLOX_BIN" init
  assert_success
  refute_output --partial "token is invalid"
  refute_output --partial "token has expired"
}

# bats test_tags=auth:pat:owner
@test "pat: flox push resolves its default owner from /me" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  # The fixture only answers a /me request bearing this test's exact secret,
  # so this also asserts the CLI sends the authorization header.
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_valid.yaml"

  "$FLOX_BIN" init --name "test"
  run "$FLOX_BIN" push
  assert_success
  assert_output --partial "owner/test"
}

# bats test_tags=auth:pat:unauthorized
@test "pat: a rejected pat warns but does not block push" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_revoked-secret"
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_revoked.yaml"

  "$FLOX_BIN" init --name "test"
  # The push itself goes to the file-based floxhub; the server stays the
  # authority for whether the token actually authenticates.
  run "$FLOX_BIN" push --owner "owner" < /dev/null
  assert_success
  assert_output --partial "could not be verified"
}

# bats test_tags=auth:pat:expired
@test "pat: an expired pat warns but does not block push" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_expired.yaml"

  "$FLOX_BIN" init --name "test"
  run "$FLOX_BIN" push --owner "owner" < /dev/null
  assert_success
  assert_output --partial "could not be verified"
}

# bats test_tags=auth:pat:degraded
@test "pat: an unreachable /me does not block calls with an explicit owner" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  # Nothing listens here: identity resolution fails, which must not be fatal.
  unset _FLOX_USE_CATALOG_MOCK
  export FLOX_CATALOG_URL="http://127.0.0.1:1"

  "$FLOX_BIN" init --name "test"
  # The push itself goes to the file-based floxhub from floxhub_setup
  # (_FLOX_FLOXHUB_GIT_URL); /me is the only HTTP dependency in this test.
  run "$FLOX_BIN" push --owner "owner"
  assert_success
  assert_output --partial "owner/test"
}

# bats test_tags=auth:pat:status
@test "pat: auth status reports the handle from /me" {
  export FLOX_FLOXHUB_TOKEN="flox_pat_test-secret"
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_valid.yaml"

  run "$FLOX_BIN" auth status
  assert_success
  assert_output --partial "You are logged in as owner"
}

# bats test_tags=auth:pat:login
@test "pat: auth login --token-file logs in with a pat" {
  unset FLOX_FLOXHUB_TOKEN
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_valid.yaml"
  echo "flox_pat_test-secret" > "$BATS_TEST_TMPDIR/token"

  run "$FLOX_BIN" auth login --token-file "$BATS_TEST_TMPDIR/token"
  assert_success
  assert_output --partial "Logged in as owner"
}

# bats test_tags=auth:pat:login:revoked
@test "pat: auth login --token-file rejects a revoked pat" {
  unset FLOX_FLOXHUB_TOKEN
  export _FLOX_USE_CATALOG_MOCK="$MANUALLY_GENERATED/auth/me_revoked.yaml"
  echo "flox_pat_revoked" > "$BATS_TEST_TMPDIR/token"

  run "$FLOX_BIN" auth login --token-file "$BATS_TEST_TMPDIR/token"
  assert_failure
  assert_output --partial "The provided token is expired."
}
