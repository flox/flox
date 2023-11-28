#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test if node stuff works with flox.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash;

# bats file_tags=end2end,node

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup;
}


# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}";
  export PROJECT_NAME="${PROJECT_DIR##*/}";
  rm -rf "$PROJECT_DIR";
  mkdir -p "$PROJECT_DIR";
  pushd "$PROJECT_DIR" >/dev/null||return;
  $FLOX_CLI init
  sed -i \
    's/from = { type = "github", owner = "NixOS", repo = "nixpkgs" }/from = { type = "github", owner = "NixOS", repo = "nixpkgs", rev = "e8039594435c68eb4f780f3e9bf3972a7399c4b1" }/' \
    "$PROJECT_DIR/.flox/env/manifest.toml";
}

project_teardown() {
  popd >/dev/null||return;
  rm -rf "${PROJECT_DIR?}";
  unset PROJECT_DIR;
  unset PROJECT_NAME;
}

# ---------------------------------------------------------------------------- #

setup()    { common_test_setup; project_setup;       }
teardown() { project_teardown; common_test_teardown; }

# ---------------------------------------------------------------------------- #
#
@test "install krb5 with node" {
  run $FLOX_CLI install nodejs krb5 pkg-config python3 gnumake clang libtool cctools;

  sed -i \
    -e 's|clang = {}|clang = { path = "clang" }|' \
      "$PROJECT_DIR/.flox/env/manifest.toml";

  assert_success;
  assert_output --partial "✅ 'nodejs' installed to environment";
  assert_output --partial "✅ 'krb5' installed to environment";
  assert_output --partial "✅ 'pkg-config' installed to environment";
  assert_output --partial "✅ 'python3' installed to environment";

  SHELL=bash run expect -d "$TESTS_DIR/end2end/node.exp" "$PROJECT_DIR";
  assert_success;
}
