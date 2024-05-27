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
}
teardown() {
  project_teardown
  common_test_teardown
}

@test "'flox install' displays confirmation message" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
}

@test "'flox install' edits manifest" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  run grep 'hello.pkg-path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml"
  assert_success
}

@test "uninstall confirmation message" {
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
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  run "$FLOX_BIN" uninstall hello
  run grep '^hello.pkg-path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml"
  assert_failure
}

@test "'flox install' reports error when package not found" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" install not-a-package
  assert_failure
  assert_output --partial "Could not find package not-a-package. Try 'flox search' with a broader search term."
}

@test "'flox install' provides suggestions when package not found" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" install package
  assert_failure
  assert_output --partial "Here are a few other similar options:"
  assert_output --partial "options with 'flox search package'"
}

@test "'flox install' doesn't provide duplicate suggestions for a multi-system environment" {
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
  "$FLOX_BIN" init
  run "$FLOX_BIN" install java
  assert_failure
  assert_output --partial "Try 'flox install jdk' instead."
  assert_output --partial "Here are a few other similar options:"
  assert_output --partial "$ flox install "
  assert_output --partial "options with 'flox search jdk'"
}

@test "'flox install' does not suggest packages if multiple packages provided" {
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
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello
  assert_success
  assert_output --partial "✅ 'hello' installed to environment"
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ]
  assert_success
}

@test "'flox uninstall' removes link to installed binary" {
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
  # If the [install] table is missing entirely we don't want to report a TOML
  # parse error, we want to report that there's nothing to uninstall.
  "$FLOX_BIN" init
  run "$FLOX_BIN" uninstall hello
  assert_failure
  assert_output --partial "couldn't uninstall 'hello', wasn't previously installed"
}

@test "'flox install' installs by path" {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'hello\.pkg-path = "hello"'
}

@test "'flox install' infers install ID" {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install rubyPackages_3_2.rails
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'rails\.pkg-path = "rubyPackages_3_2\.rails"'
}

@test "'flox install' overrides install ID with '-i'" {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install -i foo hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'foo\.pkg-path = "hello"'
}

@test "'flox install' overrides install ID with '--id'" {
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" install --id foo hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'foo\.pkg-path = "hello"'
}

@test "'flox install' accepts mix of inferred and supplied install IDs" {
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
  run "$FLOX_BIN" init
  assert_success
  run "$FLOX_BIN" i hello
  assert_success
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml")
  assert_regex "$manifest" 'hello\.pkg-path = "hello"'
}

@test "'flox install' creates global lock" {
  "$FLOX_BIN" init
  rm -f "$GLOBAL_MANIFEST_LOCK"
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" install hello
  assert_success

  # Check the expected global lock was created
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$GLOBAL_MANIFEST_LOCK"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"

  # Check the lock in the environment is the same as in the environment
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$LOCKFILE_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

@test "'flox install' uses global lock" {
  rm -f "$GLOBAL_MANIFEST_LOCK"
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_OLD?}" \
    run "$FLOX_BIN" update --global

  "$FLOX_BIN" init
  # Set new rev just to make sure we're not incidentally using old rev.
  _PKGDB_GA_REGISTRY_REF_OR_REV="${PKGDB_NIXPKGS_REV_NEW?}" \
    run "$FLOX_BIN" install hello
  assert_success

  # Check the environment used the global lock
  run jq -r '.registry.inputs.nixpkgs.from.narHash' "$LOCKFILE_PATH"
  assert_success
  assert_output "$PKGDB_NIXPKGS_NAR_HASH_OLD"
}

@test "'flox install' warns about unfree packages" {
  "$FLOX_BIN" init
  run "$FLOX_BIN" install hello-unfree
  assert_success
  assert_line --partial "The package 'hello-unfree' has an unfree license"
}

@test "'flox install' fails to install unfree packages if forbidden" {
  "$FLOX_BIN" init
  tomlq --in-place -t '.options.allow.unfree = false' "$MANIFEST_PATH"

  run "$FLOX_BIN" install hello-unfree
  assert_failure
  assert_line --partial "The package 'hello-unfree' has an unfree license."
  assert_output --partial "'options.allow.unfree = true'"
}

@test "'flox install' fails to install broken packages" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" install yi
  assert_failure
  assert_line --partial "The package 'yi' is marked as broken."
  assert_output --partial "'options.allow.broken = true'"
}

# Catalog functionality tests
# ---------------------------------------------------------------------------- #

# bats test_tags=install:catalog
@test "'flox install' installs package from catalog and builds it" {
  export FLOX_FEATURES_USE_CATALOG=true
  export _FLOX_USE_CATALOG_MOCK="$TESTS_DIR/catalog_responses/resolve/hello.json"

  "$FLOX_BIN" init
  # create a catalog manifest
  echo "version = 1" > ".flox/env/manifest.toml"
  echo 'options.systems = ["aarch64-darwin", "x86_64-darwin", "aarch64-linux", "x86_64-linux"]' >> ".flox/env/manifest.toml"

  run "$FLOX_BIN" install -i hello_install_id hello
  assert_success

  run "$FLOX_BIN" activate -- hello
  assert_success
  assert_line "Hello, world!"
}
