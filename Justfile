bats_invocation := "nix run '.#flox-tests' -- --flox target/debug/flox"
cargo_test_invocation := "cargo test --workspace"

_default:
    @just --list --unsorted

# Run the 'bats' test suite
bats-tests +test_files="":
    @cargo build
    @nix build '.#flox-bash'
    @{{bats_invocation}} {{test_files}}

# Run the Rust unit tests
unit-tests regex="":
    @{{cargo_test_invocation}} {{regex}}

# Run the test suite, including impure tests
impure-tests regex="":
    @{{cargo_test_invocation}} {{regex}} --features "extra-tests"

# Run the entire test suite, not including impure tests
test: unit-tests bats-tests

# Run the entire test suite, including impure tests
test-all: impure-tests bats-tests

# Enters the Rust development environment
work:
    @# Note that this command is only really useful if you have
    @# `just` installed outside of the `flox` environment already
    @nix develop

# Bump all flake dependencies and commit with a descriptive message
bump-all:
    @nix flake update --commit-lock-file --commit-lockfile-summary "chore: flake bump"

# Bump a specific flake input and commit with a descriptive message
bump input:
    @nix flake lock --update-input {{input}} --commit-lock-file --commit-lockfile-summary "chore: bump '{{input}}' flake input"
