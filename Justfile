bats_invocation := "flox run '.#flox-tests' -- -- --flox target/debug/flox"
cargo_test_invocation := "cargo test --workspace"

_default:
    @just --list --unsorted

# Run the 'bats' test suite
bats-tests +test_files="":
    @cargo build
    @flox build flox-bash
    @export FLOX_SH_PATH="$PWD/result"
    @export FLOX_SH="$PWD/result/libexec/flox/flox"
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
    @flox develop rust-env
