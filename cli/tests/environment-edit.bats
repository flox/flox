#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox edit
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
  export TMP_MANIFEST_PATH="${BATS_TEST_TMPDIR}/manifest.toml"

  export Hello_HOOK=$(
    cat << EOF
[hook]
script = """
  echo "Welcome to your flox environment!";
"""
EOF
  )

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  rm -f "${TMP_MANIFEST_PATH?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
  unset TMP_MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  project_setup
}
teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

check_manifest_unchanged() {
  current_contents=$(cat "$MANIFEST_PATH")
  [[ $current_contents == "$ORIGINAL_MANIFEST_CONTENTS" ]]
}

check_manifest_updated() {
  current_contents=$(cat "$MANIFEST_PATH")
  [[ $current_contents == "$NEW_MANIFEST_CONTENTS" ]]
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' confirms successful edit" {
  "$FLOX_BIN" init
  cp "$MANIFEST_PATH" "$TMP_MANIFEST_PATH"
  cat << "EOF" > "$TMP_MANIFEST_PATH"
[install]
hello.path = "hello"
EOF

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output --partial "✅ environment successfully edited"
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' says no changes made" {
  "$FLOX_BIN" init
  cp "$MANIFEST_PATH" "$TMP_MANIFEST_PATH"

  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output --partial "⚠️  no changes made to environment"
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' does not say to re-activate when hook is modified and environment is not active" {
  "$FLOX_BIN" init
  cp "$MANIFEST_PATH" "$TMP_MANIFEST_PATH"
  sed "s/\[hook\]/${HOOK//$'\n'/\\n}/" "$MANIFEST_PATH" > "$TMP_MANIFEST_PATH"


  run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output --partial "✅ environment successfully edited"
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' says to re-activate when hook is modified and environment is active" {
  "$FLOX_BIN" init

  sed "s/\[hook\]/${HOOK//$'\n'/\\n}/" "$MANIFEST_PATH" > "$TMP_MANIFEST_PATH"

  SHELL=bash run expect -d "$TESTS_DIR/edit/re-activate.exp" "$TMP_MANIFEST_PATH"
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' accepts contents via filename" {
  skip "FIXME: broken migrating to manifest.toml"
  run cat "$EXTERNAL_MANIFEST_PATH"
  run "$FLOX_BIN" edit -f "$EXTERNAL_MANIFEST_PATH"
  assert_success
  WRITTEN=$(cat "$MANIFEST_PATH")
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' accepts contents via pipe to stdin" {
  skip "FIXME: broken migrating to manifest.toml"
  run sh -c "cat ${EXTERNAL_MANIFEST_PATH} | ${FLOX_BIN} edit -f -"
  assert_success
  # Get the contents as they appear in the actual manifest after the operation
  WRITTEN=$(cat "$MANIFEST_PATH")
  # Assert that it's the same as the contents we supplied
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' fails with invalid contents supplied via filename" {
  skip "FIXME: broken migrating to manifest.toml"
  echo "foo = " > "$EXTERNAL_MANIFEST_PATH"
  run "$FLOX_BIN" edit -f "$EXTERNAL_MANIFEST_PATH"
  assert_failure
  run check_manifest_unchanged
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' fails with invalid contents supplied via stdin" {
  skip "FIXME: broken migrating to manifest.toml"
  run sh -c "echo 'foo = ;' | ${FLOX_BIN} edit -f -"
  assert_failure
  run check_manifest_unchanged
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when provided filename doesn't exist" {
  run "$FLOX_BIN" edit -f "does_not_exist.toml"
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when EDITOR is not set" {
  run "$FLOX_BIN" edit
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' adds package with EDITOR" {
  skip "FIXME: broken migrating to manifest.toml"
  EDITOR="$TESTS_DIR/add-hello" run "$FLOX_BIN" edit
  assert_success
  run check_manifest_updated
  assert_success
}

# ---------------------------------------------------------------------------- #

@test "'flox edit' fails when EDITOR makes invalid edit" {
  skip "FIXME: broken migrating to manifest.toml"
  EDITOR="$TESTS_DIR/add-invalid-edit" run "$FLOX_BIN" edit
  assert_failure
  run check_manifest_unchanged
  assert_success
}
