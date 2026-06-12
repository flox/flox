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

set positional-arguments

nix_options := "--extra-experimental-features nix-command \
                --extra-experimental-features flakes"
INPUT_DATA := "${PWD}/test_data/input_data"
TEST_DATA := "${PWD}/test_data"
cargo_test_invocation := "cargo nextest --profile ci run --workspace"

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
   meson compile -C nix-plugins/builddir --clean; \
   rm -rf build/nix-plugins


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
# `pure-eval` is disabled because `FLOX_INTERPRETER` and `FLOX_ACTIVATIONS_BIN`
# are read from the environment.
@build-buildenv:
    nix {{nix_options}} build \
        --option pure-eval false \
        ".#floxDevelopmentPackages.flox-buildenv" \
        -o "$FLOX_BUILDENV"

# ---------------------------------------------------------------------------- #
# Cargo built subsystems

# Build the flox activations binary
@build-activations:
    cargo build -p flox-activations


# Build the flox activations binary
@build-activations-release:
    cargo build -p flox-activations -r



# ---------------------------------------------------------------------------- #
# Build the flox binary

@build-cli: build-nix-plugins build-package-builder build-activation-scripts build-buildenv
    cargo build -p flox

# Build the binaries
@build: build-cli

# Build flox with release profile
@build-release: build-nix-plugins build-package-builder build-activation-scripts build-buildenv
    cargo build -p flox -r

# Remove build artifacts
@clean-builds:
    git checkout -- build/

# ---------------------------------------------------------------------------- #
# Build just the data generator

@build-data-gen:
    cargo build -p mk_data

# Generate test data
@gen-data floxhub_path +mk_data_args="": (mk-data mk_data_args)
    #!/usr/bin/env bash

    # We do this because `mk_data` has a `-f` flag whereas the
    # gen-unit-data recipe has a positional argument that can take the value
    # `force`. As far as I can tell, there's not a way to conditionally run
    # recipes within `just`, so we just run the correct recipe via a script.
    if [ "{{mk_data_args}}" = "-f" ]; then
        just gen-unit-data "{{floxhub_path}}" true
    else
        just gen-unit-data "{{floxhub_path}}"
    fi

@mk-data +mk_data_args="": build-data-gen build-cli (md mk_data_args)

# The same as mk-data, but faster to type, and doesn't rebuild stuff
@md +mk_data_args="":
    mkdata="$PWD/target/debug/mk_data"; pushd test_data; "$mkdata" {{mk_data_args}} config.toml; popd

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

    # Extract latest package versions from production catalog for test assertions.
    # These versions must match what's in the recorded mock YAML files.
    echo "Extracting latest package versions from production catalog..."
    python_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/python3' | jq -r '.items[0].version')
    go_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/go' | jq -r '.items[0].version')
    poetry_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/poetry' | jq -r '.items[0].version')
    nodejs_20_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/nodejs_20' | jq -r '.items[0].version')
    jq -n \
        --arg python3 "$python_version" \
        --arg go "$go_version" \
        --arg poetry "$poetry_version" \
        --arg nodejs_20 "$nodejs_20_version" \
        '{python3: $python3, go: $go, poetry: $poetry, nodejs_20: $nodejs_20}' \
        > "{{TEST_DATA}}/unit_test_generated/latest_prod_versions.json"
    echo "Wrote latest_prod_versions.json with python3=$python_version, go=$go_version, poetry=$poetry_version, nodejs_20=$nodejs_20_version"

gen-unit-data-for-publish floxhub_repo_path force="":
    #!/usr/bin/env bash

    # Use local services for publish tests, must already be running.
    # In the FloxHub repo, run:
    # flox activate -- just catalog-server::serve-all

    set -euo pipefail

    # Get the catalog server URL from the FloxHub environment
    catalog_server_url="$(flox activate -d "{{floxhub_repo_path}}" -- bash -c 'echo $FLOXHUB_CATALOG_SERVER_URL')"

    # Get the latest Nixpkgs revision that exists in the catalog
    nixpkgs_rev="$(curl -X 'GET' --silent "${catalog_server_url}/info/base-catalog" -H 'accept: application/json' | jq .scraped_pages[0].rev | tr -d "'\"")"
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

@gen-unit-data floxhub_path force="false": (gen-unit-data-no-publish force) (gen-unit-data-for-publish floxhub_path force)

# ---------------------------------------------------------------------------- #

# Generate JSON schemas for Flox data structures
@gen-schemas:
  cargo xtask generate-schemas

# ---------------------------------------------------------------------------- #

# Run the nix-plugins tests
@test-nix-plugins: build-nix-plugins
    meson test -C nix-plugins/builddir

# Run the CLI integration test suite using locally built binaries
# This is equivalent to the "local" jobs in CI.
@integ-tests *bats_args: build
    flox-cli-tests "$@"

# Run the CLI integration test suite using Nix-built binaries
# This is equivalent to the "remote" jobs in CI.
@nix-integ-tests *bats_args:
    nix run \
        --accept-flake-config \
        --extra-experimental-features 'nix-command flakes' \
        .#flox-cli-tests -- "$@"

@ut regex="" record="false":
    _FLOX_UNIT_TEST_RECORD={{record}} {{cargo_test_invocation}} {{regex}}

# Run the CLI unit tests
@unit-tests regex="" record="false": build (ut regex record)

build-nef-test-fixtures:
    #!/usr/bin/env bash
    set -euo pipefail
    cargo build --package nef-lock-catalog 1>&2
    lock_bin="$PWD/target/debug/lock"
    testdata="$PWD/package-builder/nef/tests/instantiateTests/testData"
    tmpdir=$(realpath "$(mktemp -d "${TMPDIR:-/tmp}/nef-test-fixtures.XXXXXX")")
    cp -r "$testdata"/* "$tmpdir/"
    # Lock deepest configs first so child locks exist before parents reference them
    find "$tmpdir" -name nix-builds.toml -print0 \
        | sort -zr \
        | while IFS= read -r -d '' config; do
            dir=$(dirname "$config")
            (cd "$dir" && "$lock_bin" nix-builds.toml)
          done
    echo "$tmpdir"

test-nef:
    #!/usr/bin/env bash
    set -euo pipefail
    fixtures="$(just build-nef-test-fixtures)"
    nix-unit package-builder/nef/tests \
        --arg nixpkgs-url "$COMMON_NIXPKGS_URL" \
        --argstr test-fixtures "$fixtures"

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

# Refresh the prior-release lockfile fixtures used by AI-159 cross-release
# tests.  Fixtures live in
#   test_data/manually_generated/prior_release_baselines/
#
# This recipe is documentation-of-procedure and automation for future
# maintainers.  The fixtures checked into the repository are the actual test
# inputs; this recipe is only needed when it is time to advance the prior
# release pin.
#
# Usage:
#   just regen-prior-release-fixtures           # auto: picks N-1 minor
#   just regen-prior-release-fixtures 1.12.0    # explicit version
#
# When to run:
#   - A new minor Flox release ships (advance the pin to the new N-1)
#   - The lockfile schema bumps (fixture format changed)
#   - A predicate-rejection test fails with a fixture-rot diagnostic
#
# The recipe is intentionally left as a commented-out shell script rather
# than a runnable recipe, because it requires:
#   1. A prior Flox release binary (not available in CI)
#   2. Network access to the Flox catalog
#   3. A real Nix store for the build stamp
#
# To run manually:
#
#   PRIOR_VERSION="${1:-auto}"
#   BASELINES="test_data/manually_generated/prior_release_baselines"
#   FIXTURES_DIR="$BASELINES/PENDING_CAPTURE"
#
#   # Step 1: obtain prior Flox binary
#   #   nix profile install github:flox/flox/$PRIOR_VERSION
#   #   PRIOR_FLOX=$(which flox)
#
#   # Step 2: lock and build each fixture shape
#   #   for shape in plain with_include; do
#   #     WORK=$(mktemp -d)
#   #     cp "$FIXTURES_DIR/$shape/manifest.toml" "$WORK/manifest.toml"
#   #     cd "$WORK"
#   #     $PRIOR_FLOX init
#   #     cp manifest.toml .flox/env/manifest.toml
#   #     FLOX_CATALOG_DUMP="$PWD/replay.yaml" $PRIOR_FLOX activate -c true
#   #     cp .flox/env/manifest.lock "$FIXTURES_DIR/$shape/manifest.lock"
#   #     cp replay.yaml "$FIXTURES_DIR/$shape/catalog_replay.yaml"
#   #   done
#
#   # Step 3: update MANIFEST.json with version, date, sha256s
#   # Step 4: git add and commit with message naming the new version
#
# See test_data/manually_generated/prior_release_baselines/README.md for
# the full procedure.
regen-prior-release-fixtures version="auto":
    #!/usr/bin/env bash
    set -euo pipefail
    echo "regen-prior-release-fixtures: see Justfile comments for the manual"
    echo "procedure. Automatic fixture capture requires a prior Flox binary"
    echo "and network access to the Flox catalog."
    echo ""
    echo "Version requested: {{version}}"
    echo ""
    echo "Full procedure documented in:"
    echo "  test_data/manually_generated/prior_release_baselines/README.md"
    exit 1


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
     cargo metadata --format-version 1              \
       |jq -r '.packages[]|[.name,.license]|@csv';


# ---------------------------------------------------------------------------- #

# Run a `flox` command
@flox +args="": build
    target/debug/flox {{args}}

# Run a `flox` command using the catalog
@catalog-flox +args="": build
    echo "just: DEPRECATED TARGET: Use 'flox' instead" >&2;
    target/debug/flox {{args}}


# ---------------------------------------------------------------------------- #

# Clean ( remove ) built artifacts
@clean: clean-nix-plugins
    cargo clean

# ---------------------------------------------------------------------------- #

@format-cli:
    cargo fmt

@format-nix-plugins:
    clang-format -i nix-plugins/src/**/*.cc; \
    clang-format -i nix-plugins/include/**/*.hh

@format-nix:
    treefmt -f nix

# format yaml files (i.e.e github actions)
@format-yaml:
    treefmt -f yaml

# Format all the code
format: format-cli format-nix-plugins format-nix format-yaml
# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
