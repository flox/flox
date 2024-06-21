#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox install`
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=install

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
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.json"
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "'flox install' displays confirmation message" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
}

@test "'flox install' edits manifest" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  run grep 'hello.pkg-path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml"
  assert_success
}

@test "uninstall confirmation message" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"

  run "$FLOX_BIN" uninstall hello
  assert_success
  # Note that there's TWO spaces between the emoji and the package name
  assert_output --partial "🗑️  'hello' uninstalled from environment"
}

@test "'flox uninstall' edits manifest" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  run "$FLOX_BIN" uninstall hello
  run grep '^hello.pkg-path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml"
  assert_failure
}

@test "'flox install' provides suggestions when package not found" {
  "$FLOX_BIN" init
  # This package doesn't exist but *does* have suggestions
  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/package_suggestions.json" \
    run "$FLOX_BIN" install package
  assert_failure
  assert_output --partial "Here are a few other similar options:"
  assert_output --partial "options with 'flox search package'"
}

@test "'flox install' doesn't provide duplicate suggestions for a multi-system environment" {
  skip "will be fixed by https://github.com/flox/flox/issues/1482"
  rm -f "$GLOBAL_MANIFEST_LOCK"

  "$FLOX_BIN" init
  # add a second system
  tomlq -i -t ".options.systems += [ \"$(get_system_other_than_current)\" ]" "$MANIFEST_PATH"
  run "$FLOX_BIN" install npm
  assert_failure
  # TODO: it would be less lazy to assert 3 distinct packages are returned
  # rather than hardcoding package names.
  assert_output --partial "flox install nodejs"
  assert_output --partial "flox install elmPackages.nodejs"
  assert_output --partial "flox install nodePackages.nodejs"
}

@test "'flox install' provides curated suggestions when package not found" {
  skip "will be fixed by https://github.com/flox/flox/issues/1482"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install java
  assert_failure
  assert_output --partial "Try 'flox install jdk' instead."
  assert_output --partial "Here are a few other similar options:"
  assert_output --partial "$ flox install "
  assert_output --partial "options with 'flox search jdk'"
}

@test "'flox install' does not suggest packages if multiple packages provided" {
  skip "will be fixed by https://github.com/flox/flox/issues/1482"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install java make
  assert_failure
  assert_output --partial "Could not install java, make"
}

@test "'flox uninstall' reports error when package not found" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" uninstall not-a-package
  assert_failure
  assert_output --partial "couldn't uninstall 'not-a-package', wasn't previously installed"
}

@test "'flox install' creates link to installed binary" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ]
  assert_success
}

@test "'flox uninstall' removes link to installed binary" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ]
  assert_success
  run "$FLOX_BIN" uninstall hello
  assert_success
  run [ ! -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ]
  assert_success
}

@test "'flox uninstall' has helpful error message with no packages installed" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  # If the [install] table is missing entirely we don't want to report a TOML
  # parse error, we want to report that there's nothing to uninstall.
  "$FLOX_BIN" init
  run "$FLOX_BIN" uninstall hello
  assert_failure
  assert_output --partial "couldn't uninstall 'hello', wasn't previously installed"
}

@test "'flox install' installs by path" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'hello\.pkg-path = "hello"'
}

@test "'flox install' infers install ID" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/rubyPackages_3_2.rails.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install rubyPackages_3_2.rails
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'rails\.pkg-path = "rubyPackages_3_2\.rails"'
}

@test "'flox install' overrides install ID with '-i'" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install -i foo hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'foo\.pkg-path = "hello"'
}

@test "'flox install' overrides install ID with '--id'" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install --id foo hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'foo\.pkg-path = "hello"'
}

@test "'flox install' accepts mix of inferred and supplied install IDs" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/webmention_ripgrep_rails.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install -i foo rubyPackages_3_2.webmention ripgrep -i bar rubyPackages_3_2.rails
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'foo\.pkg-path = "rubyPackages_3_2\.webmention"'
  assert_regex "$manifest" 'ripgrep\.pkg-path = "ripgrep"'
  assert_regex "$manifest" 'bar\.pkg-path = "rubyPackages_3_2\.rails"'
}

@test "'flox i' aliases to 'install'" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello.json"
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" i hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'hello\.pkg-path = "hello"'
}

# This is also checking we can build an unfree package
@test "'flox install' warns about unfree packages" {
  "$FLOX_BIN" init
  export  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello_unfree.json"
  run "$FLOX_BIN" install hello-unfree
  assert_success
  assert_line --partial "The package 'hello-unfree' has an unfree license"
}

@test "'flox install' warns about broken packages" {
  skip "waiting for broken packages to be added to catalog"
  "$FLOX_BIN" init
  run "$FLOX_BIN" install TODO
  assert_success
  assert_line --partial "The package 'TODO' is marked as broken, it may not behave as expected during runtime"
}

@test "resolution message: single package not found, without curation" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/badpkg.json" \
    run "$FLOX_BIN" install badpkg

  assert_failure
  assert_output "$(cat <<EOF
❌ ERROR: resolution failed: Could not find package 'badpkg'.
Try 'flox search' with a broader search term.
EOF
)"
}

@test "resolution message: multiple packages not found, without curation" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/badpkg1_badpkg2.json" \
    run "$FLOX_BIN" install badpkg1 badpkg2

  assert_failure
  assert_output "$(cat <<EOF
❌ ERROR: resolution failed: multiple resolution failures:
- Could not find package 'badpkg1'.
  Try 'flox search' with a broader search term.
- Could not find package 'badpkg2'.
  Try 'flox search' with a broader search term.
EOF
)"
}

@test "resolution message: single package not found, with curation" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/node_suggestions.json" \
    run "$FLOX_BIN" install node

  assert_failure
  assert_output --partial "$(cat <<EOF
❌ ERROR: resolution failed: Could not find package 'node'.
Try 'flox install nodejs' instead.

Here are a few other similar options:
  $ flox install nodejs
EOF
)"
}

@test "resolution message: single package not availabe on all systems" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/bpftrace.json" \
    run "$FLOX_BIN" install bpftrace

  assert_failure
  assert_output "$(cat <<EOF
❌ ERROR: resolution failed: package 'bpftrace' not available for
    - aarch64-darwin
    - x86_64-darwin
  but it is available for
    - aarch64-linux
    - x86_64-linux

For more on managing system-specific packages, visit the documentation:
https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages
EOF
)"
}

@test "resolution message: multiple packages not available on all systems" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/bpftrace_systemd.json" \
    run "$FLOX_BIN" install bpftrace systemd

  assert_failure
  assert_output "$(cat <<EOF
❌ ERROR: resolution failed: multiple resolution failures:
- package 'bpftrace' not available for
      - aarch64-darwin
      - x86_64-darwin
    but it is available for
      - aarch64-linux
      - x86_64-linux

  For more on managing system-specific packages, visit the documentation:
  https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages
- package 'systemd' not available for
      - aarch64-darwin
      - x86_64-darwin
    but it is available for
      - aarch64-linux
      - x86_64-linux

  For more on managing system-specific packages, visit the documentation:
  https://flox.dev/docs/tutorials/multi-arch-environments/#handling-unsupported-packages
EOF
)"
}

@test "resolution message: constraints too tight" {
  "$FLOX_BIN" init

  _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/old_node.json" \
    run "$FLOX_BIN" install nodejs@14.16.1

  assert_failure
  assert_output "$(cat <<EOF
❌ ERROR: resolution failed: constraints for group 'toplevel' are too tight

   Use 'flox edit' to adjust version constraints in the [install] section,
   or isolate dependencies in a new group with '<pkg>.pkg-group = "newgroup"'
EOF
)"
}
