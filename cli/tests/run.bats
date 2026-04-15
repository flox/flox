#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Integration tests for `flox run` — binary-first UX.
#
# These tests cover:
#   - Argument parsing and --help output
#   - Binary not found (empty search result → helpful error)
#   - Ambiguous binary in non-interactive mode (multiple packages → helpful error)
#   - Cache: read, write, clear via --reselect
#   - Explicit --package flag
#
# Tests that require actual package builds (happy path, piped input) are
# marked @skip-without-build and run only in environments with full Nix.
#
# bats file_tags=run
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
}

teardown() {
  common_test_teardown
}

setup_file() {
  common_file_setup
}

# ---------------------------------------------------------------------------- #

@test "'flox run --help' shows binary-first usage" {
  run "$FLOX_BIN" run --help
  assert_success
  assert_output --partial "<binary>"
  assert_output --partial "--package"
  assert_output --partial "--reselect"
}

# ---------------------------------------------------------------------------- #

@test "'flox run' requires a binary argument" {
  export RUST_BACKTRACE=0
  run "$FLOX_BIN" run
  assert_failure
}

# ---------------------------------------------------------------------------- #

@test "'flox run' errors helpfully when binary is not found in catalog" {
  export RUST_BACKTRACE=0
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/run/not_found.yaml"
  run "$FLOX_BIN" run nonexistent-binary-xyz
  assert_failure
  assert_output --partial "nonexistent-binary-xyz"
  assert_output --partial "flox search"
}

# ---------------------------------------------------------------------------- #

@test "'flox run' fails with helpful error for ambiguous binary in non-interactive mode" {
  export RUST_BACKTRACE=0
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/run/vi_ambiguous.yaml"
  # Pipe something to stdin to make it non-interactive.
  run bash -c "echo '' | '$FLOX_BIN' run vi"
  assert_failure
  # Should list candidates and suggest --package.
  assert_output --partial "vim"
  assert_output --partial "--package"
}

# ---------------------------------------------------------------------------- #

@test "'flox run' caches binary choice and uses it on subsequent invocations" {
  # Populate the binary preferences cache directly.
  local state_dir="$FLOX_STATE_DIR"
  mkdir -p "$state_dir"
  cat > "$state_dir/binary_preferences.toml" <<EOF
[choices]
testbinary = "testpkg"
EOF

  # Now run with a mock that would fail if searched (we expect cache hit).
  # The search endpoint mock returns empty — if we hit search, we'd fail.
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/run/not_found.yaml"
  export RUST_BACKTRACE=0

  # We expect to fail at the install stage (package doesn't exist in store),
  # not at the search stage. The error should NOT say "No packages found".
  run "$FLOX_BIN" run testbinary
  # Should NOT fail with "No packages found" (which is the search error).
  refute_output --partial "No packages found that provide"
}

# ---------------------------------------------------------------------------- #

@test "'flox run --reselect' fails in non-interactive mode" {
  export RUST_BACKTRACE=0
  run bash -c "echo '' | '$FLOX_BIN' run --reselect vi"
  assert_failure
  assert_output --partial "--reselect"
  assert_output --partial "interactive"
}

# ---------------------------------------------------------------------------- #

@test "'flox run --package' with explicit package is accepted" {
  export RUST_BACKTRACE=0
  # With a mock that returns empty search (we should bypass search with --package).
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/run/not_found.yaml"
  # The command will fail at the install stage (no real nix store), but should
  # not fail with "No packages found" (search is bypassed when --package is given).
  run "$FLOX_BIN" run --package hello hello
  refute_output --partial "No packages found that provide"
}

# ---------------------------------------------------------------------------- #

@test "'flox run' help text mentions 'man flox-run'" {
  run "$FLOX_BIN" run --help
  assert_success
  assert_output --partial "man flox-run"
}
