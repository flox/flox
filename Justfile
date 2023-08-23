flox_bats_tests_invocation := "flox run '.#flox-tests' -- -- --flox target/debug/flox"

_default:
    @just --list --unsorted

# Run the 'bats' test suite
shell-test +test_files="":
    @cargo build
    @{{flox_bats_tests_invocation}} {{test_files}}

# Run the Rust unit tests
unit-test +regex="":
    @cargo test --workspace {{regex}}

# Run the entire test suite
test: unit-test shell-test
