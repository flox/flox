#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test rust impl of `flox install`
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test";
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
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

# without specifying a name should install to an environment found in the user's current directory.
@test "i2.a: install outside of shell (option1)" {
  skip "Environment defaults handled in another phase"
}

@test "flox install allows -r for installing to a specific remote environment name, creating a new generation." {
  skip "remote environments handled in another phase"
}

@test "'flox install' displays confirmation message" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install hello;
  assert_success;
  assert_output --partial "✅ 'hello' installed to environment";
}

@test "'flox install' edits manifest" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install hello;
  assert_success;
  run grep 'hello.path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml";
  assert_success;
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
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install hello;
  assert_success;
  run "$FLOX_BIN" uninstall hello;
  run grep '^hello.path = "hello"' "$PROJECT_DIR/.flox/env/manifest.toml";
  assert_failure;
}

@test "'flox install' reports error when package not found" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install not-a-package;
  assert_failure;
  assert_output --partial "failed to resolve \`not-a-package'";
}

@test "'flox uninstall' reports error when package not found" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" uninstall not-a-package;
  assert_failure;
  assert_output --partial "couldn't uninstall 'not-a-package', wasn't previously installed";
}

@test "'flox install' creates link to installed binary" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install hello;
  assert_success;
  assert_output --partial "✅ 'hello' installed to environment";
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ];
  assert_success;
}

@test "'flox uninstall' removes link to installed binary" {
  "$FLOX_BIN" init;
  run "$FLOX_BIN" install hello;
  assert_success;
  assert_output --partial "✅ 'hello' installed to environment";
  run [ -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ];
  assert_success;
  run "$FLOX_BIN" uninstall hello;
  assert_success;
  run [ ! -e "$PROJECT_DIR/.flox/run/$NIX_SYSTEM.$PROJECT_NAME/bin/hello" ];
  assert_success;
}

@test "'flox uninstall' has helpful error message with no packages installed" {
  # If the [install] table is missing entirely we don't want to report a TOML
  # parse error, we want to report that there's nothing to uninstall.
  "$FLOX_BIN" init;
  run "$FLOX_BIN" uninstall hello;
  assert_failure;
  assert_output --partial "couldn't uninstall 'hello', wasn't previously installed";
}

@test "'flox install' uses last activated environment" {
  mkdir 1
  "$FLOX_BIN" init --dir 1

  mkdir 2
  "$FLOX_BIN" init --dir 2

  SHELL=bash NO_COLOR=1 run expect -d "$TESTS_DIR/install/last-activated.exp"
  assert_success
}

@test "'flox install' prompts when an environment is activated and there is an environment in the current directory" {
  mkdir 1
  "$FLOX_BIN" init --dir 1

  mkdir 2
  "$FLOX_BIN" init --dir 2

  SHELL=bash NO_COLOR=1 run -0 expect -d "$TESTS_DIR/install/prompt-which-environment.exp"
}

@test "'flox install' prompts when an environment is activated and there is an environment in the containing git repo" {
  mkdir 1
  "$FLOX_BIN" init --dir 1

  mkdir 2
  "$FLOX_BIN" init --dir 2
  git -C 2 init
  mkdir 2/subdirectory

  SHELL=bash NO_COLOR=1 run -0 expect -d "$TESTS_DIR/install/prompt-which-environment-git.exp"
}

@test "i5: download package when install command runs" {
  skip "Don't know how to test, check out-link created?"
}

@test "i6: install on a pushed environment stages locally" {
  skip "remote environments handled in another phase"
}

@test "'flox install' installs by path" {
  run "$FLOX_BIN" init;
  assert_success;
  run "$FLOX_BIN" install hello;
  assert_success;
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml");
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'hello\.path = "hello"';
}

@test "'flox install' infers install ID" {
  run "$FLOX_BIN" init;
  assert_success;
  run "$FLOX_BIN" install rubyPackages_3_2.rails;
  assert_success;
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml");
  # This also checks that it correctly infers the install ID
  assert_regex "$manifest" 'rails\.path = "rubyPackages_3_2\.rails"';
}

@test "'flox install' overrides install ID with '-i'" {
  run "$FLOX_BIN" init;
  assert_success;
  run "$FLOX_BIN" install -i foo hello;
  assert_success;
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml");
  assert_regex "$manifest" 'foo\.path = "hello"';
}

@test "'flox install' overrides install ID with '--id'" {
  run "$FLOX_BIN" init;
  assert_success;
  run "$FLOX_BIN" install --id foo hello;
  assert_success;
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml");
  assert_regex "$manifest" 'foo\.path = "hello"';
}

@test "'flox install' accepts mix of inferred and supplied install IDs" {
  run "$FLOX_BIN" init;
  assert_success;
  run "$FLOX_BIN" install -i foo rubyPackages_3_2.webmention ripgrep -i bar rubyPackages_3_2.rails;
  assert_success;
  manifest=$(cat "$PROJECT_DIR/.flox/env/manifest.toml");
  assert_regex "$manifest" 'foo\.path = "rubyPackages_3_2\.webmention"';
  assert_regex "$manifest" 'ripgrep\.path = "ripgrep"';
  assert_regex "$manifest" 'bar\.path = "rubyPackages_3_2\.rails"';
}
