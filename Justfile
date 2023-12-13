# ============================================================================ #
#
# Think of this as a `Makefile' except you run `just <TARGET>' instead
# of `make <TARGET>'.
#
# The difference between `just' and `make' is that `just' does not check
# timestamps to determine if files are stale; so in that sense you can imagine
# it as `make' except "all targets are `.PHONY' targets".
#
#
# ---------------------------------------------------------------------------- #

nix_options := "--extra-experimental-features nix-command --extra-experimental-features flakes"
cargo_test_invocation := "cargo test --workspace"


# ---------------------------------------------------------------------------- #

_default:
    @just --list --unsorted


# ---------------------------------------------------------------------------- #

build-pkgdb:
    @pushd pkgdb; make -j; popd

build-env-builder: build-pkgdb
    @pushd env-builder; make -j; popd

build-cli: build-env-builder
    @pushd cli; cargo build; popd

# Build the binaries
build: build-cli


# ---------------------------------------------------------------------------- #

test-pkgdb: build-pkgdb
    @pushd pkgdb; make -j tests; popd
    @pushd pkgdb; make check; popd

test-env-builder: build-env-builder
    @pushd env-builder; make -j tests; popd
    @pushd env-builder; make check; popd

# Run the end-to-end test suite
functional-tests +bats_args="": build
    @flox-tests {{bats_args}}

# Run the integration test suite
integ-tests: build
    @flox-cli-tests --pkgdb "${PWD}/pkgdb/bin/pkgdb" --flox "${PWD}/cli/target/debug/flox" --env-builder "${PWD}/env-builder/bin/env-builder"

# Run a specific 'bats' test file
bats-file file: build
    @flox-tests --tests "{{file}}"

# Run the Rust unit tests
unit-tests regex="": build
    @pushd cli; {{cargo_test_invocation}} {{regex}}; popd

# Run the test suite, including impure tests
impure-tests regex="": build
    @pushd cli; {{cargo_test_invocation}} {{regex}} --features "extra-tests"; popd

# Run the entire test suite, not including impure tests
test-cli: build unit-tests functional-tests integ-tests

# Run the entire test suite, including impure tests
test-all: test-pkgdb test-env-builder impure-tests functional-tests integ-tests


# ---------------------------------------------------------------------------- #

# Enters the Rust development environment
work:
    @# Note that this command is only really useful if you have
    @# `just` installed outside of the `flox` environment already
    @nix {{nix_options}} develop


# ---------------------------------------------------------------------------- #

# Bump all flake dependencies and commit with a descriptive message
bump-all:
    @nix {{nix_options}} flake update --commit-lock-file --commit-lockfile-summary "chore: flake bump"

# Bump a specific flake input and commit with a descriptive message
bump input:
    @nix {{nix_options}} flake lock --update-input {{input}} --commit-lock-file --commit-lockfile-summary "chore: bump '{{input}}' flake input"


# ---------------------------------------------------------------------------- #

# Output licenses of all dependency crates
license:
    @pushd cli; cargo metadata --format-version 1 | jq -r '.packages[] | [.name, .license] | @csv'


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
