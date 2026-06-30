#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Integration tests for `flox run`
#
# Tests that require a real Nix store download are tagged `run:store`
# and can be run with:
#   just integ-tests run.bats -- --filter-tags run:store
#
# All other tests use the catalog mock mechanism and run without network.
#
# ---------------------------------------------------------------------------- #

load test_support.bash
# bats file_tags=run

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
}

teardown() {
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# Help
# ---------------------------------------------------------------------------- #

@test "'flox run --help' shows synopsis" {
  run "$FLOX_BIN" run --help
  assert_success
  assert_output --partial "flox run"
  assert_output --partial "-p"
}

@test "'flox help run' shows synopsis without panic" {
  run "$FLOX_BIN" help run
  assert_success
  assert_output --partial "flox run"
  assert_output --partial "-p"
}

# ---------------------------------------------------------------------------- #
# Required-flag errors (no network or store required)
# ---------------------------------------------------------------------------- #

@test "'flox run' with no args reports missing package" {
  run "$FLOX_BIN" run
  assert_failure
  assert_output --partial "No package specified"
}

@test "'flox run <command>' without -p reports missing package" {
  run "$FLOX_BIN" run cowsay
  assert_failure
  assert_output --partial "No package specified"
}

@test "'flox run -p' without a value reports missing package value" {
  run "$FLOX_BIN" run -p
  assert_failure
  assert_output --partial "Missing value"
}

# ---------------------------------------------------------------------------- #
# Unknown-flag handling
# ---------------------------------------------------------------------------- #

@test "'flox run --unknown-flag <cmd>' before command reports unknown option" {
  run "$FLOX_BIN" run --unknown-flag curl
  assert_failure
  assert_output --partial "Unknown option"
  assert_output --partial "'--'"
}

# ---------------------------------------------------------------------------- #
# Unsupported package spec rejection (no network required)
# ---------------------------------------------------------------------------- #

@test "'flox run' rejects version constraint (@) in package spec" {
  run "$FLOX_BIN" run -p "hello@2.12" hello
  assert_failure
  assert_output --partial "Unsupported package"
}

@test "'flox run' rejects output selector (^) in package spec" {
  run "$FLOX_BIN" run -p "foo^bin" foo
  assert_failure
  assert_output --partial "Unsupported package"
}

@test "'flox run' rejects custom catalog (/) in package spec" {
  run "$FLOX_BIN" run -p "mycat/vim" vi
  assert_failure
  assert_output --partial "Unsupported package"
}

# ---------------------------------------------------------------------------- #
# Resolution errors (mock catalog, no store required)
# ---------------------------------------------------------------------------- #

@test "'flox run' reports package not found with search hint" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/failed_resolution.yaml"
  run "$FLOX_BIN" run -p nonexistent-xyz-package nonexistent-xyz-package
  assert_failure
  assert_output --partial "not found"
  assert_output --partial "flox search"
}

# ---------------------------------------------------------------------------- #
# Help passthrough regression (regression for prototype defect 1)
#
# With the original prototype, `flox run -p curl --help` would incorrectly
# show flox's help because --help was intercepted anywhere in argv.
# With the OsString state machine, --help after the command stays in
# passthrough.  The cheap version of this test exercises resolution failure
# (which happens before exec, so we can assert without a store), while the
# store variant actually execs the command.
# ---------------------------------------------------------------------------- #

@test "help-passthrough: '--help' after command is NOT shown as flox help (cheap)" {
  # Using a mock that returns not-found: if --help were intercepted by flox,
  # this would exit 0 (flox help) instead of failing with "not found".
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/failed_resolution.yaml"
  run "$FLOX_BIN" run -p nonexistent-xyz-package nonexistent-xyz-package --help
  assert_failure
  assert_output --partial "not found"
  # Must NOT show flox run's own help text
  refute_output --partial "flox run -p"
}

# ---------------------------------------------------------------------------- #
# Store tests (require Nix binary cache access)
#
# Run with:  just integ-tests run.bats -- --filter-tags run:store
# Skip with: just integ-tests run.bats -- --filter-tags '!run:store'
# ---------------------------------------------------------------------------- #

# bats test_tags=run:store
@test "flox run -p hello hello executes and succeeds [run:store]" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  run "$FLOX_BIN" run -p hello hello
  assert_success
  assert_output --partial "Hello"
}

# bats test_tags=run:store
@test "flox run -p hello hello -g 'hi flox' passes args verbatim [run:store]" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  run "$FLOX_BIN" run -p hello hello -g "hi flox"
  assert_success
  assert_output --partial "hi flox"
}

# bats test_tags=run:store
@test "flox run exit code is forwarded: hello --bogus-flag fails [run:store]" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  run "$FLOX_BIN" run -p hello hello --bogus-flag
  assert_failure
}

# bats test_tags=run:store
@test "flox run -- hello --version passes --version to command [run:store]" {
  # Without `--`, flox intercepts --version (Version::check scans all args).
  # With `--`, --version reaches the command.
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  run "$FLOX_BIN" run -p hello -- hello --version
  assert_success
  # GNU hello prints its version to stdout
  assert_output --partial "hello"
}

# bats test_tags=run:store
@test "flox run -p hello -- hello --help shows hello's help [run:store]" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  run "$FLOX_BIN" run -p hello -- hello --help
  assert_success
  # GNU Hello's help mentions its own name
  assert_output --partial "hello"
  # Must NOT show flox run's own synopsis
  refute_output --partial "flox run -p"
}

# ---------------------------------------------------------------------------- #
# Source-build fallback (require Nix access to attempt the build)
# ---------------------------------------------------------------------------- #

# bats test_tags=run:store
@test "flox run falls back to source build when binary not in cache [run:store]" {
  # terraform is unfree; the mock returns a store path that won't exist in
  # the local Nix store, so substitution fails and the source-build path is
  # taken. Since stderr is not a TTY in bats, no prompt is shown and the
  # build proceeds automatically.
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/terraform.yaml"
  run "$FLOX_BIN" run -p terraform -- terraform --version
  assert_success
  assert_output --partial "Terraform"
}

# bats test_tags=run:store
@test "stable GC root: second run -p hello hello skips download [run:store]" {
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/hello.yaml"
  # First run: populates the GC root.
  run "$FLOX_BIN" run -p hello hello
  assert_success

  # GC root should exist under $FLOX_CACHE_DIR/run-gc-roots/
  local gc_dir="${FLOX_CACHE_DIR:-$HOME/.cache/flox}/run-gc-roots"
  run ls "$gc_dir"
  assert_success
  # At least one entry containing "hello" in its name
  assert_output --partial "hello"

  # Second run: still succeeds (store path already present / cached).
  run "$FLOX_BIN" run -p hello hello
  assert_success
}

# bats test_tags=run:store
@test "source-build GC root is removed after the command exits [run:store]" {
  # terraform is unfree and not in the local cache, so 'flox run' builds it
  # from source and forks a watcher that removes the per-PID 'build-*' GC root
  # once the exec'd command exits. The "Terraform" output confirms the
  # source-built binary actually ran, so the cleanup assertion below applies
  # to a real source build rather than passing vacuously.
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/run/terraform.yaml"
  run "$FLOX_BIN" run -p terraform -- terraform --version
  assert_success
  assert_output --partial "Terraform"

  local gc_dir="${FLOX_CACHE_DIR:-$HOME/.cache/flox}/run-gc-roots"

  # The watcher polls getppid() every 500ms, so the build GC root may linger
  # briefly after the command exits. Poll for its removal (up to ~10s).
  local tries=0
  while true; do
    shopt -s nullglob
    local leftovers=("$gc_dir"/build-*)
    shopt -u nullglob
    [[ ${#leftovers[@]} -eq 0 ]] && break

    tries=$((tries + 1))
    if [[ $tries -gt 100 ]]; then
      echo "ERROR: build GC root not cleaned up after ~10s: ${leftovers[*]}" >&3
      return 1
    fi
    sleep 0.1
  done
}
