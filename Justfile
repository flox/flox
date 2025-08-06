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
INPUT_DATA := "${PWD}/test_data/input_data"
TEST_DATA := "${PWD}/test_data"
cargo_test_invocation := "cargo nextest --profile ci run --manifest-path ${PWD}/cli/Cargo.toml --workspace"

# Set the FLOX_VERSION variable so that it can be used in the build/runtime
# It's important to add the git revision to the version string,
# so to that `containerize` can build the correct version of flox in CI.
# While technically we'd want to add `-dirty` to the version string if the
# working directory is dirty, we omit this here because in practice
# it causes tests to fail that expect a FLAKE_VERSION to be "clean",
# and doesn't add practical information.
export FLOX_VERSION := shell('cat ./VERSION') + "-g" + shell('git rev-parse --short HEAD')

# ---------------------------------------------------------------------------- #

@_default:
    just --list --unsorted

# ---------------------------------------------------------------------------- #

version:
    echo "${FLOX_VERSION}"

# ---------------------------------------------------------------------------- #

# Print the paths of all of the binaries
@bins:
    echo "$FLOX_BIN"


# ---------------------------------------------------------------------------- #
# Build Nix plugins

# Build only nix-plugins
@build-nix-plugins:
    meson compile -C nix-plugins/builddir; \
    meson install -C nix-plugins/builddir

# Clean the nix-plugins build cache
@clean-nix-plugins:
   meson compile -C nix-plugins/builddir --clean


# ---------------------------------------------------------------------------- #
# Nix built subsystems

# Build the flox manpages
@build-manpages:
    nix {{nix_options}} build .#flox-manpages -o build/flox-manpages

# Build the activation scripts
# `pure-eval` is disabled because `FLOX_ACTIVATIONS_BIN`
# is read from the environment.
@build-activation-scripts: build-activations
    nix {{nix_options}} build \
        --option pure-eval false \
        '.#floxDevelopmentPackages.flox-interpreter^*' \
        -o $FLOX_INTERPRETER

# Build the flox package builder
@build-package-builder:
    nix {{nix_options}} build \
        ".#floxDevelopmentPackages.flox-package-builder" \
        -o "$FLOX_PACKAGE_BUILDER"

# Build the flox buildenv
# `pure-eval` is disabled because `FLOX_INTERPRETER`
# is read from the environment.
@build-buildenv:
    nix {{nix_options}} build \
        --option pure-eval false \
        ".#floxDevelopmentPackages.flox-buildenv" \
        -o "$FLOX_BUILDENV"

# ---------------------------------------------------------------------------- #
# Cargo built subsystems

# Build the flox activations binary
@build-activations:
    pushd cli; cargo build -p flox-activations

# Build the flox watchdog binary
@build-watchdog:
    pushd cli; cargo build -p flox-watchdog

# Build the flox activations binary
@build-activations-release:
    pushd cli; cargo build -p flox-activations -r

# Build the flox watchdog binary
@build-watchdog-release:
    pushd cli; cargo build -p flox-watchdog -r


# ---------------------------------------------------------------------------- #
# Build the flox binary

@build-cli: build-nix-plugins build-package-builder build-activation-scripts build-watchdog build-buildenv
    pushd cli; cargo build -p flox

# Build the binaries
@build: build-cli

# Build flox with release profile
@build-release: build-nix-plugins build-package-builder build-activation-scripts build-watchdog-release build-buildenv
    pushd cli; cargo build -p flox -r

# Remove build artifacts
@clean-builds:
    git checkout -- build/

# ---------------------------------------------------------------------------- #
# Build just the data generator

@build-data-gen:
    pushd cli; cargo build -p mk_data; popd

# Generate test data
@gen-data floxhub_path +mk_data_args="": (mk-data mk_data_args)
    #!/usr/bin/env bash

    # We do this because `mk_data` has a `-f` flag whereas the
    # gen-unit-data recipe has a positional argument that can take the value
    # `force`. As far as I can tell, there's not a way to conditionally run
    # recipes within `just`, so we just run the correct recipe via a script.
    if [ "{{mk_data_args}}" = "-f" ]; then
        just gen-unit-data "{{floxhub_path}}" force
    else
        just gen-unit-data "{{floxhub_path}}"
    fi

@mk-data +mk_data_args="": build-data-gen build-cli md

# The same as mk-data, but faster to type, and doesn't rebuild stuff
@md +mk_data_args="":
    mkdata="$PWD/cli/target/debug/mk_data"; pushd test_data; "$mkdata" {{mk_data_args}} config.toml; popd

gen-unit-data-no-publish force="":
    #!/usr/bin/env bash

    set -e

    if [ "{{force}}" = "true" ]; then
        export _FLOX_UNIT_TEST_RECORD="force"
    else
        export _FLOX_UNIT_TEST_RECORD="missing"
    fi

    # Use remote services for non-publish tests
    {{cargo_test_invocation}} --filterset 'not (test(providers::build::tests) | test(providers::publish) | test(commands::publish) | test(providers::catalog::tests::creates_new_catalog))'

gen-unit-data-for-publish floxhub_repo_path force="":
    #!/usr/bin/env bash

    # Use local services for publish tests, must already be running.
    # In the FloxHub repo, run:
    # flox activate -- just catalog-server::serve-all

    set -euo pipefail

    # Get the latest Nixpkgs revision that exists in the catalog
    nixpkgs_rev="$(curl -X 'GET' --silent 'http://localhost:8000/api/v1/catalog/info/base-catalog' -H 'accept: application/json' | jq .scraped_pages[0].rev | tr -d "'\"")"
    if [ -z "$nixpkgs_rev" ]; then
        echo "failed to communicate with floxhub services"
        exit 1
    fi
    echo "$nixpkgs_rev" > "{{TEST_DATA}}/unit_test_generated/latest_dev_catalog_rev.txt"

    # Grab configuration variables from the FloxHub repo's environment
    # (Only needed if you want to use Auth0 instead of the test users)
    # export _FLOX_OAUTH_AUTH_URL="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $_FLOX_OAUTH_AUTH_URL')"
    # export _FLOX_OAUTH_TOKEN_URL="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $_FLOX_OAUTH_TOKEN_URL')"
    # export _FLOX_OAUTH_DEVICE_AUTH_URL="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $_FLOX_OAUTH_DEVICE_AUTH_URL')"
    # export _FLOX_OAUTH_CLIENT_ID="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $_FLOX_OAUTH_CLIENT_ID')"
    export FLOX_CONFIG_DIR="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $FLOX_CONFIG_DIR')"
    export _FLOXHUB_TEST_USER_ROLES="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $_FLOXHUB_TEST_USER_ROLES')"
    # We need this test user info persistent when we run the tests.
    cp $_FLOXHUB_TEST_USER_ROLES "{{TEST_DATA}}/floxhub_test_users.json"

    # Set the recording variable based on Justfile arguments
    export _FLOX_UNIT_TEST_RECORD=true
    if [ "{{force}}" = "true" ]; then
        export _FLOX_UNIT_TEST_RECORD="force"
    else
        export _FLOX_UNIT_TEST_RECORD="missing"
    fi

    # Run the tests that will regenerate the mocks
    {{cargo_test_invocation}} --no-fail-fast --filterset 'test(providers::publish) | test(commands::publish) | test(providers::catalog::tests::creates_new_catalog)'

@gen-unit-data floxhub_path: gen-unit-data-no-publish (gen-unit-data-for-publish floxhub_path)

# ---------------------------------------------------------------------------- #

# Run the nix-plugins tests
@test-nix-plugins: build-nix-plugins
    meson test -C nix-plugins/builddir

# Run the CLI integration test suite using locally built binaries
# This is equivalent to the "local" jobs in CI.
@integ-tests +bats_args="": build
    flox-cli-tests \
        {{bats_args}}

# Run the CLI integration test suite using Nix-built binaries
# This is equivalent to the "remote" jobs in CI.
@nix-integ-tests +bats_args="":
    nix run \
        --accept-flake-config \
        --extra-experimental-features 'nix-command flakes' \
        .#flox-cli-tests \
        {{bats_args}}

@ut regex="" record="false":
    _FLOX_UNIT_TEST_RECORD={{record}} {{cargo_test_invocation}} {{regex}}

# Run the CLI unit tests
@unit-tests regex="" record="false": build (ut regex record)

test-nef:
    nix-unit package-builder/nef/tests --arg nixpkgs-url "$COMMON_NIXPKGS_URL"

test-buildenvLib:
    nix-unit buildenv/buildenvLib/tests

# Run the CLI unit tests, including impure tests
@impure-tests regex="": build
     {{cargo_test_invocation}} {{regex}} --features "extra-tests"

# Run the entire CLI test suite
test-cli: impure-tests integ-tests

# Run the test suite except for nix-plugins
@test-rust: impure-tests integ-tests nix-integ-tests

# Run the entire test suite, including impure unit tests
test-all: test-nix-plugins impure-tests integ-tests nix-integ-tests


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


# ---------------------------------------------------------------------------- #

# Clean ( remove ) built artifacts
@clean: clean-nix-plugins
    pushd cli; cargo clean; popd

# ---------------------------------------------------------------------------- #

@format-cli:
    pushd cli; cargo fmt; popd

@format-nix-plugins:
    clang-format -i nix-plugins/src/**/*.cc; \
    clang-format -i nix-plugins/include/**/*.hh


@format-nix:
    treefmt

# Format all the code
format: format-cli format-nix-plugins format-nix

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
