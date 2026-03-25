#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox trust
#
# bats file_tags=trust
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Helper: count files in a directory (excluding . and ..)
count_files() {
  local dir="$1"
  if [ ! -d "$dir" ]; then
    echo 0
  else
    find "$dir" -maxdepth 1 -type f | wc -l | tr -d ' '
  fi
}

# ---------------------------------------------------------------------------- #

@test "'flox trust' succeeds on initialized environment" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" trust
  assert_success
  assert_output --partial "Trusted environment"
  assert_output --partial "for auto-activation"

  # Verify trust file exists
  [ "$(count_files "$FLOX_DATA_DIR/trust/allowed")" -ge 1 ]
}

@test "'flox trust --deny' creates deny file" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" trust --deny
  assert_success
  assert_output --partial "Denied auto-activation"

  # Verify deny file exists
  [ "$(count_files "$FLOX_DATA_DIR/trust/denied")" -ge 1 ]
}

@test "'flox trust' fails when no .flox exists" {
  # No flox init — empty PROJECT_DIR
  run "$FLOX_BIN" trust
  assert_failure
  assert_output --partial "No '.flox' environment found"
}

@test "'flox trust --path' trusts a specific directory" {
  mkdir -p subdir
  pushd subdir > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  run "$FLOX_BIN" trust --path subdir
  assert_success
  assert_output --partial "Trusted environment"
}

@test "'flox trust' after '--deny' revokes denial" {
  "$FLOX_BIN" init

  "$FLOX_BIN" trust --deny
  [ "$(count_files "$FLOX_DATA_DIR/trust/denied")" -ge 1 ]

  # Trust should remove the deny file
  run "$FLOX_BIN" trust
  assert_success

  [ "$(count_files "$FLOX_DATA_DIR/trust/denied")" -eq 0 ]
}

@test "'flox trust --deny' after trust creates deny file" {
  "$FLOX_BIN" init

  "$FLOX_BIN" trust
  [ "$(count_files "$FLOX_DATA_DIR/trust/allowed")" -ge 1 ]

  run "$FLOX_BIN" trust --deny
  assert_success

  [ "$(count_files "$FLOX_DATA_DIR/trust/denied")" -ge 1 ]
}

@test "'flox init' automatically trusts new environment" {
  run "$FLOX_BIN" init
  assert_success

  # init should auto-trust the environment
  [ "$(count_files "$FLOX_DATA_DIR/trust/allowed")" -ge 1 ]
}

# bats test_tags=trust:install
@test "'flox install' re-trusts after manifest change" {
  "$FLOX_BIN" init

  local count_before
  count_before="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" install hello

  local count_after
  count_after="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  # Install changes the manifest, so a new trust hash is created.
  # The old trust file is orphaned (still present), so count increases.
  [ "$count_after" -gt "$count_before" ]
}

# bats test_tags=trust:uninstall
@test "'flox uninstall' re-trusts after manifest change" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  "$FLOX_BIN" install hello

  local count_before
  count_before="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  "$FLOX_BIN" uninstall hello

  local count_after
  count_after="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  [ "$count_after" -gt "$count_before" ]
}

# bats test_tags=trust:edit
@test "'flox edit' re-trusts after manifest change" {
  "$FLOX_BIN" init

  local count_before
  count_before="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  # Write a modified manifest to a temp file with a comment change
  local tmp_manifest="${BATS_TEST_TMPDIR}/manifest.toml"
  with_latest_schema '
# This is an edit to change the manifest
[options]
' > "$tmp_manifest"

  "$FLOX_BIN" edit -f "$tmp_manifest"

  local count_after
  count_after="$(count_files "$FLOX_DATA_DIR/trust/allowed")"

  [ "$count_after" -gt "$count_before" ]
}

# bats test_tags=trust:hook-env
@test "manual manifest edit revokes trust (verified via hook-env)" {
  "$FLOX_BIN" init

  # Verify it's currently trusted
  local allowed_count
  allowed_count="$(count_files "$FLOX_DATA_DIR/trust/allowed")"
  [ "$allowed_count" -ge 1 ]

  # Manually edit the manifest (bypassing flox commands — no re-trust)
  echo '# modified' >> "$MANIFEST_PATH"

  # hook-env should report the environment as not trusted
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "not trusted" "$BATS_TEST_TMPDIR/stderr"
}
