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
cargo_test_invocation := "cargo nextest run --manifest-path ${PWD}/cli/Cargo.toml --workspace"

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
@gen-data +mk_data_args="": build-data-gen build-cli
    mkdata="$PWD/cli/target/debug/mk_data"; pushd test_data; "$mkdata" {{mk_data_args}} config.toml; popd


# ---------------------------------------------------------------------------- #

# Run the nix-plugins tests
@test-nix-plugins: build-nix-plugins
    meson test -C nix-plugins/builddir

# Run the CLI integration test suite
@integ-tests +bats_args="": build
    flox-cli-tests \
        --nix-plugins "$NIX_PLUGINS" \
        --flox "$FLOX_BIN" \
        --watchdog "$WATCHDOG_BIN" \
        --input-data "{{INPUT_DATA}}" \
        --generated-data "$GENERATED_DATA" \
        {{bats_args}}

# Run the CLI integration test suite using Nix-built binaries
@nix-integ-tests:
    nix run \
        --accept-flake-config \
        --extra-experimental-features 'nix-command flakes' \
        .#flox-cli-tests

# Run the CLI unit tests
@unit-tests regex="": build
     {{cargo_test_invocation}} {{regex}}

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
