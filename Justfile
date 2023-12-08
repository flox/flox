nix_options := "--extra-experimental-features nix-command --extra-experimental-features flakes"
cargo_test_invocation := "cargo test --workspace"

_default:
    @just --list --unsorted

# Build the binaries
build:
    @cargo build -q
    @pushd pkgdb; make -j -s; popd
    @pushd env-builder; make -j -s; popd

# Run the 'bats' test suite
bats-tests +bats_args="": build
    @flox-tests {{bats_args}}

# Run a specific 'bats' test file
bats-file file: build
    @flox-tests --tests "{{file}}"

# Run the Rust unit tests
unit-tests regex="": build
    @{{cargo_test_invocation}} {{regex}}

# Run the test suite, including impure tests
impure-tests regex="": build
    @{{cargo_test_invocation}} {{regex}} --features "extra-tests"

# Run the entire test suite, not including impure tests
test: build unit-tests bats-tests

# Run the entire test suite, including impure tests
test-all: build impure-tests bats-tests

# Enters the Rust development environment
work:
    @# Note that this command is only really useful if you have
    @# `just` installed outside of the `flox` environment already
    @nix {{nix_options}} develop

# Bump all flake dependencies and commit with a descriptive message
bump-all:
    @nix {{nix_options}} flake update --commit-lock-file --commit-lockfile-summary "chore: flake bump"

# Bump a specific flake input and commit with a descriptive message
bump input:
    @nix {{nix_options}} flake lock --update-input {{input}} --commit-lock-file --commit-lockfile-summary "chore: bump '{{input}}' flake input"

# Output licenses of all dependency crates
license:
    @cargo metadata --format-version 1 | jq -r '.packages[] | [.name, .license] | @csv'
