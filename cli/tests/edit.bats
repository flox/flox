#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox edit
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=edit

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
}

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
  export TMP_MANIFEST_PATH="${BATS_TEST_TMPDIR}/manifest.toml"
  export EXTERNAL_MANIFEST_PATH="${TESTS_DIR}/edit/manifest.toml"

  export Hello_HOOK=$(
    cat << EOF
[hook]
on-activate = """
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
  setup_isolated_flox
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
version = 1
[install]
hello.pkg-path = "hello"
EOF

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" edit -f "$TMP_MANIFEST_PATH"
  assert_success
  assert_output "✅ Environment successfully updated."
}

# ---------------------------------------------------------------------------- #

# bats test_tags=edit:manifest:file
@test "'flox edit' accepts contents via filename" {
  NEW_MANIFEST_CONTENTS="$(cat "$EXTERNAL_MANIFEST_PATH")"
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" edit -f "$EXTERNAL_MANIFEST_PATH"
  assert_success

  WRITTEN="$(cat "$MANIFEST_PATH")"
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=edit:manifest:stdin
@test "'flox edit' accepts contents via pipe to stdin" {
  NEW_MANIFEST_CONTENTS="$(cat "$EXTERNAL_MANIFEST_PATH")"
  "$FLOX_BIN" init

  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run sh -c "cat ${EXTERNAL_MANIFEST_PATH} | ${FLOX_BIN} edit -f -"
  assert_success
  # Get the contents as they appear in the actual manifest after the operation
  WRITTEN="$(cat "$MANIFEST_PATH")"
  # Assert that it's the same as the contents we supplied
  assert_equal "$WRITTEN" "$NEW_MANIFEST_CONTENTS"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=edit:manifest:file:invalid
@test "'flox edit' fails with invalid contents supplied via filename" {

  "$FLOX_BIN" init
  ORIGINAL_MANIFEST_CONTENTS="$(cat "$MANIFEST_PATH")" # for check_manifest_unchanged

  cat "$EXTERNAL_MANIFEST_PATH" > ./manifest.toml
  echo "foo = " > ./manifest.toml

  run "$FLOX_BIN" edit -f ./manifest.toml
  assert_failure
  run check_manifest_unchanged
  assert_success
}

# ---------------------------------------------------------------------------- #
# bats test_tags=edit:manifest:stdin:invalid
@test "'flox edit' fails with invalid contents supplied via stdin" {

  "$FLOX_BIN" init
  ORIGINAL_MANIFEST_CONTENTS="$(cat "$MANIFEST_PATH")" # for check_manifest_unchanged

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

# bats test_tags=edit:rename
@test "'flox edit --name' edits .flox/env.json" {
  "$FLOX_BIN" init --name "before"

  BEFORE="$(jq -r .name .flox/env.json)"
  assert_equal "$BEFORE" "before"

  run "$FLOX_BIN" edit --name "after"
  assert_success

  AFTER="$(jq -r .name .flox/env.json)"
  assert_equal "$AFTER" "after"
}

# bats test_tags=edit:rename-remote
@test "'flox edit --name' fails with a remote environment" {
  floxhub_setup "owner"

  "$FLOX_BIN" init --name name
  "$FLOX_BIN" push --owner "owner"

  run "$FLOX_BIN" edit --remote "owner/name" --name "renamed"
  assert_failure
  assert_output --partial "Cannot rename environments on FloxHub"
}

# ---------------------------------------------------------------------------- #

# bats test_tags=edit:unchanged
@test "'flox edit' returns if it does not detect changes" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json" \
    run "$FLOX_BIN" edit -f "$TESTS_DIR/edit/manifest.toml"
  assert_success

  # applying the same edit again should return early
  # (simulates quiting the editor without saving)
  run "$FLOX_BIN" edit -f "$TESTS_DIR/edit/manifest.toml"
  assert_success
  assert_output "⚠️  No changes made to environment."

}

# ---------------------------------------------------------------------------- #

# bats test_tags=edit:priority
@test "'flox edit' priority" {
  "$FLOX_BIN" init

WITHOUT_PRIORITY=$(cat <<EOF
version = 1
[install]
vim.pkg-path = "vim"
vim-full.pkg-path = "vim-full"
EOF
)

WITH_PRIORITY=$(cat <<EOF
version = 1
[install]
vim.pkg-path = "vim"
vim-full.pkg-path = "vim-full"
vim-full.priority = 4
EOF
)
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/vim-vim-full-conflict.json"
  run "$FLOX_BIN" edit -f <(echo "$WITHOUT_PRIORITY")
  assert_failure

  run "$FLOX_BIN" edit -f <(echo "$WITH_PRIORITY")
  assert_success

  run "$FLOX_BIN" edit -f <(echo "$WITHOUT_PRIORITY")
  assert_failure
}
