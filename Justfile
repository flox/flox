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

nix_options := "--extra-experimental-features nix-command \
 --extra-experimental-features flakes"
PKGDB_BIN := "${PWD}/pkgdb/bin/pkgdb"
FLOX_BIN := "${PWD}/cli/target/debug/flox"
KLAUS_BIN := "${PWD}/cli/target/debug/klaus"
cargo_test_invocation := "PKGDB_BIN=${PKGDB_BIN} cargo nextest run --manifest-path ${PWD}/cli/Cargo.toml --workspace"


# ---------------------------------------------------------------------------- #

@_default:
    just --list --unsorted


# ---------------------------------------------------------------------------- #

# Print the paths of all of the binaries
@bins:
    echo "{{PKGDB_BIN}}"
    echo "{{FLOX_BIN}}"

# ---------------------------------------------------------------------------- #

# Build the compilation database
build-cdb:
    @make -C pkgdb -j 8 -s cdb

# Build only pkgdb
@build-pkgdb:
    make -C pkgdb -j 8;

# Build pkgdb with debug symbols
@build-pkgdb-debug:
    # Note that you need to clean pkgdb first
    make -C pkgdb -j 8 -s DEBUG=1

# Clean the pkgdb build cache
@clean-pkgdb:
    make -C pkgdb -j 8 -s clean

# Build only flox
@build-cli: build-pkgdb
    pushd cli; cargo build -q --workspace

# Build just the data generator
@build-data-gen:
    pushd cli; cargo build -q -p mk_data; popd

# Build the binaries
build: build-cli

# Generate test data
@gen-data +mk_data_args="": build-data-gen
    mkdata="$PWD/cli/target/debug/mk_data"; pushd test_data; "$mkdata" {{mk_data_args}} config.toml; popd

# ---------------------------------------------------------------------------- #

# Run the pkgdb tests
@test-pkgdb: build-pkgdb
    make -C pkgdb -j 8 tests;
    make -C pkgdb check;

# Run the CLI integration test suite
@integ-tests +bats_args="": build
    flox-cli-tests \
        --pkgdb "{{PKGDB_BIN}}" \
        --flox "{{FLOX_BIN}}" \
        --klaus "{{KLAUS_BIN}}" \
        {{bats_args}}

# Run the CLI unit tests
@unit-tests regex="": build
     {{cargo_test_invocation}} {{regex}}

# Run the CLI unit tests, including impure tests
@impure-tests regex="": build
     {{cargo_test_invocation}} {{regex}} --features "extra-tests"

# Run the entire CLI test suite
test-cli: impure-tests integ-tests

# Run the entire test suite, including impure unit tests
test-all: test-pkgdb impure-tests integ-tests


# ---------------------------------------------------------------------------- #

# Enters the Rust development environment
@work:
    # Note that this command is only really useful if you have
    # `just` installed outside of the `flox` environment already
    nix {{nix_options}} develop


# ---------------------------------------------------------------------------- #

# Bump all flake dependencies and commit with a descriptive message
@bump-all:
    nix {{nix_options}} flake update --commit-lock-file  \
         --commit-lockfile-summary "chore: flake bump";

# Bump a specific flake input and commit with a descriptive message
@bump input:
    nix {{nix_options}} flake lock --update-input {{input}}  \
         --commit-lock-file --commit-lockfile-summary         \
         "chore: bump '{{input}}' flake input";


# ---------------------------------------------------------------------------- #

# Output licenses of all dependency crates
@license:
    pushd cli;                                     \
     cargo metadata --format-version 1              \
       |jq -r '.packages[]|[.name,.license]|@csv';


# ---------------------------------------------------------------------------- #

# Run a `flox` command
@flox +args="": build
    cli/target/debug/flox {{args}}

# Run a `flox` command using the catalog
@catalog-flox +args="": build
    echo "just: DEPRECATED TARGET: Use 'flox' instead" >&2;
    cli/target/debug/flox {{args}}

@pkgdb-flox +args="": build
    FLOX_FEATURES_USE_CATALOG=false cli/target/debug/flox {{args}}

# Run a `pkgdb` command
@pkgdb +args="": build-pkgdb
    pkgdb/bin/pkgdb {{args}}


# ---------------------------------------------------------------------------- #

# Clean ( remove ) built artifacts
@clean:
    pushd cli; cargo clean;
    make -C pkgdb clean;


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
