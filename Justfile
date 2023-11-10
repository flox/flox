nix_options := "--extra-experimental-features nix-command --extra-experimental-features flakes"
cargo_test_invocation := "cargo test --workspace"
bats_invocation := "nix --extra-experimental-features nix-command --extra-experimental-features flakes run '.#flox-tests' -- --flox target/debug/flox"

_default:
    @just --list --unsorted

# Run the 'bats' test suite
bats-tests +bats_args="":
    @cargo build -q
    @{{bats_invocation}} -- {{bats_args}}

# Run a specific 'bats' test file
bats-file file:
    @cargo build -q
    @{{bats_invocation}} "{{file}}"

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
    @nix {{nix_options}} develop

# Bump all flake dependencies and commit with a descriptive message
bump-all:
    @nix {{nix_options}} flake update --commit-lock-file --commit-lockfile-summary "chore: flake bump"

# Bump a specific flake input and commit with a descriptive message
bump input:
    @nix {{nix_options}} flake lock --update-input {{input}} --commit-lock-file --commit-lockfile-summary "chore: bump '{{input}}' flake input"
