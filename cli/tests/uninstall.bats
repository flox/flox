#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox uninstall`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=uninstall

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  export LOCKFILE_PATH="$PROJECT_DIR/.flox/env/manifest.lock"
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset LOCKFILE_PATH
  unset MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "uninstall: confirmation message" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output "✔ 'hello' installed to environment 'test'"

  run "$FLOX_BIN" uninstall hello
  assert_success
  # Note that there's TWO spaces between the emoji and the package name
  assert_output "━ 'hello' uninstalled from environment 'test'"
}

@test "uninstall: errors (without proceeding) for already uninstalled packages" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success

  # disable backtrace; we expect this to fail and assert output
  RUST_BACKTRACE=0 run "$FLOX_BIN" uninstall hello curl
  assert_failure
  assert_output "✘ ERROR: no package named 'curl' in the manifest"
}

@test "uninstall: edits manifest" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  run "$FLOX_BIN" uninstall hello
  run grep '^hello.pkg-path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml"
  assert_failure
}

@test "uninstall: reports error when package not found" {
  "$FLOX_BIN" init
  # disable backtrace; we expect this to fail and assert output
  RUST_BACKTRACE=0 run "$FLOX_BIN" uninstall not-a-package
  assert_failure
  assert_output "✘ ERROR: no package named 'not-a-package' in the manifest"
}

@test "uninstall: removes link to installed binary" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✔ 'hello' installed to environment"
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/bin/hello" ]
  assert_success
  run "$FLOX_BIN" uninstall hello
  assert_success
  run [ ! -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME.dev/bin/hello" ]
  assert_success
}

@test "uninstall: has helpful error message with no packages installed" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml"
  # If the [install] table is missing entirely we don't want to report a TOML
  # parse error, we want to report that there's nothing to uninstall.
  "$FLOX_BIN" init
  # disable backtrace; we expect this to fail and assert output
  RUST_BACKTRACE=0 run "$FLOX_BIN" uninstall hello
  assert_failure
  assert_output "✘ ERROR: no package named 'hello' in the manifest"
}

@test "uninstall: can uninstall packages with dotted att_paths" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/rubyPackages_3_2.rails.yaml"
  run "$FLOX_BIN" init
  assert_success
  # Install a dotted package
  run "$FLOX_BIN" install rubyPackages_3_2.rails
  assert_success

  # The package should be in the manifest
  manifest_after_install=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest_after_install" 'rails\.pkg-path = "rubyPackages_3_2\.rails"'

  # Flox can uninstall the dotted package
  run "$FLOX_BIN" uninstall rubyPackages_3_2.rails
  assert_success

  # The package should be removed from the manifest
  manifest_after_uninstall=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  ! assert_regex "$manifest_after_uninstall" 'rails\.pkg-path = "rubyPackages_3_2\.rails"'
}

@test "uninstall: removes a package from a composing environment" {
  "$FLOX_BIN" init -d included
  "$FLOX_BIN" edit -d included -f - <<- EOF
version = 1
EOF

  "$FLOX_BIN" init -d composer
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -d composer -f - <<- EOF
version = 1

[install]
hello.pkg-path = "hello"

[include]
environments = [
  { dir = "../included" },
]
EOF

  run "$FLOX_BIN" uninstall -d composer hello
  assert_success
  assert_output "━ 'hello' uninstalled from environment 'composer'"

  run "$FLOX_BIN" list -d composer
  assert_success
  assert_output - << EOF
! No packages are installed for your current system ('${NIX_SYSTEM}').

You can see the whole manifest with 'flox list --config'.
EOF
}

@test "uninstall: refuses to remove a package from an included environment" {
  "$FLOX_BIN" init -d included
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -d included -f - <<- EOF
version = 1

[install]
hello.pkg-path = "hello"
EOF

  "$FLOX_BIN" init -d composer
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -d composer -f - <<- EOF
version = 1

[include]
environments = [
  { dir = "../included" },
]
EOF

  # disable backtrace; we expect this to fail and assert output
  RUST_BACKTRACE=0 \
    run "$FLOX_BIN" uninstall -d composer hello
  assert_failure
  assert_output - << EOF
✘ ERROR: Cannot remove included package 'hello'
Remove the package from environment 'included' and then run 'flox include upgrade'
EOF
}

@test "uninstall: warns when removing a package that is still provided by an include" {
  "$FLOX_BIN" init -d included
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -d included -f - <<- EOF
version = 1

[install]
hello.pkg-path = "hello"
EOF

  "$FLOX_BIN" init -d composer
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.yaml" \
    "$FLOX_BIN" edit -d composer -f - <<- EOF
version = 1

[install]
hello.pkg-path = "hello"

[include]
environments = [
  { dir = "../included" },
]
EOF

  run "$FLOX_BIN" uninstall -d composer hello
  assert_success
  assert_output - << EOF
━ 'hello' uninstalled from environment 'composer'
ℹ 'hello' is still installed by environment 'included'
EOF
}
