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

set positional-arguments := true

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
    nix {{ nix_options }} build .#flox-manpages -o build/flox-manpages

# Build the activation scripts
# `pure-eval` is disabled because `FLOX_ACTIVATIONS_BIN`

# is read from the environment.
@build-activation-scripts: build-activations
    nix {{ nix_options }} build \
        --option pure-eval false \
        '.#floxDevelopmentPackages.flox-interpreter^*' \
        -o $FLOX_INTERPRETER

# Build the flox package builder
@build-package-builder:
    nix {{ nix_options }} build \
        ".#floxDevelopmentPackages.flox-package-builder" \
        -o "$FLOX_PACKAGE_BUILDER"

# Build the flox buildenv
# `pure-eval` is disabled because `FLOX_INTERPRETER` and `FLOX_ACTIVATIONS_BIN`

# are read from the environment.
@build-buildenv:
    nix {{ nix_options }} build \
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

    # `mk_data` (run via the mk-data dependency above) and the unit-cassette
    # recorder below must agree on whether this is a force run. `mk_data`
    # treats `-f`/`--force` as force, so mirror exactly that set here. Reject
    # any other argument loudly: silently falling back to a replay run records
    # nothing and yields an empty mocks patch that masquerades as success.
    case "{{ mk_data_args }}" in
        -f | --force)
            just gen-unit-data "{{ floxhub_path }}" true
            ;;
        "")
            just gen-unit-data "{{ floxhub_path }}"
            ;;
        *)
            echo "gen-data: cannot thread '{{ mk_data_args }}' through to the unit-cassette recorder." >&2
            echo "Only '-f'/'--force' (force-record all mocks) or no argument (record only missing mocks) are understood here." >&2
            exit 1
            ;;
    esac

@mk-data +mk_data_args="": build-data-gen build-cli (md mk_data_args)

# The same as mk-data, but faster to type, and doesn't rebuild stuff
@md +mk_data_args="":
    mkdata="$PWD/target/debug/mk_data"; (cd test_data && "$mkdata" {{ mk_data_args }} config.toml)

gen-unit-data-no-publish force="":
    #!/usr/bin/env bash

    set -euo pipefail

    # `_FLOX_RESOLVE_STABILITY` / `FloxhubClientConfig::stability` (this PR)
    # is mechanism only — NOT pinned yet, deliberately no export here. There
    # are two committed cassette stores (`test_data/unit_test_generated/`,
    # replayed by `auto_recording_client_inner`; and
    # `test_data/generated/resolve/`, replayed by `catalog_replay_client`),
    # and neither was recorded with a stability key. httpmock's playback
    # matcher matches on the recorded `when` body, so setting this var
    # anywhere it reaches a record or replay path before both stores are
    # regenerated together would strand cassette matches in one direction
    # or the other.
    #
    # The first LTS pin lands as a single atomic future change, gated on
    # HUB-119 (flox/floxhub#1908) reaching production:
    #   1. Set _FLOX_RESOLVE_STABILITY=lts here, in
    #      test_data/config.toml [vars], and in the test-running/replay
    #      contexts (unit-tests, ut, impure-tests).
    #   2. Flip `catalog_replay_client` (cli/flox-rust-sdk/src/providers/
    #      catalog.rs) from hardcoded `stability: None` to
    #      `FloxhubClientConfig::stability_from_env()`.
    #      `auto_recording_client_inner` already reads the env var and
    #      needs no change.
    #   3. Force-regenerate both cassette stores: unit_test_generated via
    #      `just gen-unit-data-no-publish force=true`, and the mk_data
    #      store via `just md`.
    #   4. Verify both replay suites are green.

    if [ "{{ force }}" = "true" ]; then
        export _FLOX_UNIT_TEST_RECORD="force"

        # Refresh the version baseline from the production catalog so it stays
        # in lockstep with the mocks being recorded in this run. Several init
        # tests assert the resolved version against these values
        # (via latest_prod_versions.json). Replay mode leaves the committed
        # baseline untouched so that replayed mocks and the committed baseline
        # remain consistent.
        echo "Extracting latest package versions from production catalog..."
        python_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/python3' | jq -r '.items[0].version')
        go_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/go' | jq -r '.items[0].version')
        poetry_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/poetry' | jq -r '.items[0].version')
        nodejs_20_version=$(curl -s 'https://api.flox.dev/api/v1/catalog/packages/nodejs_20' | jq -r '.items[0].version')

        versions_file="{{ TEST_DATA }}/unit_test_generated/latest_prod_versions.json"
        jq -n \
            --arg python3 "$python_version" \
            --arg go "$go_version" \
            --arg poetry "$poetry_version" \
            --arg nodejs_20 "$nodejs_20_version" \
            '{python3: $python3, go: $go, poetry: $poetry, nodejs_20: $nodejs_20}' \
            > "$versions_file"

        # A failed fetch leaves an empty or "null" string. Reject a baseline
        # with any unresolved version by checking the written file, which
        # covers every package it contains.
        if jq -e 'any(.[]; . == "" or . == "null")' "$versions_file" >/dev/null; then
            echo "failed to resolve a package version from the production catalog" >&2
            exit 1
        fi
        echo "Wrote latest_prod_versions.json with python3=$python_version, go=$go_version, poetry=$poetry_version, nodejs_20=$nodejs_20_version"
    else
        export _FLOX_UNIT_TEST_RECORD="missing"
    fi

    # Use remote services for non-publish tests
    {{ cargo_test_invocation }} --filterset 'not (test(providers::build::tests) | test(providers::publish) | test(commands::publish) | test(providers::catalog::tests::creates_new_catalog))'

# Reset the local FloxHub catalog DB to clean dump state via floxhub's
# catalog-updater db-reset recipe. Requires the floxhub stack to be running.
# Also useful standalone before ad-hoc recording sessions, e.g.:
#   just reset-floxhub-db ../floxhub && just ut 'providers::publish' true
@reset-floxhub-db floxhub_repo_path:
    echo "Resetting catalog DB to clean dump state..."
    flox activate -d "{{ floxhub_repo_path }}" -- bash -c \
        'cd "{{ floxhub_repo_path }}" && just catalog-updater db-reset'

gen-unit-data-for-publish floxhub_repo_path force="": (reset-floxhub-db floxhub_repo_path)
    #!/usr/bin/env bash

    # Publish tests need the local FloxHub stack already running, started from
    # the repo ROOT env (auth0+jwt):
    #   flox activate -- just catalog-server::serve-for-mocks
    #
    # Record only against that ROOT env — recording needs auth0+jwt. The
    # on-prem envs/floxhub deployment uses reverse-proxy auth that ignores
    # bearer tokens; serve-for-mocks refuses to start unless FLOXHUB_AUTHN_MODE
    # is auth0, so a stack started the documented way is always correct.
    #
    # reset-floxhub-db gives every run a clean DB dump, so any running stack
    # works with no manual teardown.

    set -euo pipefail

    # Refresh the dev catalog baseline only when force-recording so the
    # committed latest_dev_catalog_rev.txt stays in lockstep with the mocks
    # recorded in this run. Replay leaves it untouched, mirroring the prod
    # version baseline in gen-unit-data-no-publish.
    if [ "{{ force }}" = "true" ]; then
        # Read the latest Nixpkgs rev from the catalog. reset-floxhub-db has
        # already run, so this reads the fresh dump (never a stale pre-reset
        # DB) against a DB-ready server; keep the reset ahead of this fetch or
        # the committed rev and recorded mocks drift out of lockstep.
        #
        # Fetch via the floxhub env to reuse its canonical
        # FLOXHUB_CATALOG_SERVER_URL (one source for the /api/v1 path, no
        # hardcoded version) and its mkcert CA trust. The || true lets a
        # connection failure fall through to the diagnostic below instead of
        # aborting under set -e.
        base_catalog_info="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'curl -X GET --silent "$FLOXHUB_CATALOG_SERVER_URL/info/base-catalog" -H "accept: application/json"' || true)"
        nixpkgs_rev="$(jq .scraped_pages[0].rev <<< "$base_catalog_info" | tr -d "'\"" || true)"
        if [ -z "$nixpkgs_rev" ]; then
            echo "failed to communicate with floxhub services"
            exit 1
        fi
        echo "$nixpkgs_rev" > "{{ TEST_DATA }}/unit_test_generated/latest_dev_catalog_rev.txt"
    fi

    # Grab configuration variables from the FloxHub repo's environment
    # (Only needed if you want to use Auth0 instead of the test users)
    # export _FLOX_OAUTH_AUTH_URL="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $_FLOX_OAUTH_AUTH_URL')"
    # export _FLOX_OAUTH_TOKEN_URL="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $_FLOX_OAUTH_TOKEN_URL')"
    # export _FLOX_OAUTH_DEVICE_AUTH_URL="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $_FLOX_OAUTH_DEVICE_AUTH_URL')"
    # export _FLOX_OAUTH_CLIENT_ID="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $_FLOX_OAUTH_CLIENT_ID')"
    export FLOX_CONFIG_DIR="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $FLOX_CONFIG_DIR')"
    export _FLOXHUB_TEST_USER_ROLES="$(flox activate -d "{{ floxhub_repo_path }}" -- bash -c 'echo $_FLOXHUB_TEST_USER_ROLES')"
    # We need this test user info persistent when we run the tests.
    cp $_FLOXHUB_TEST_USER_ROLES "{{ TEST_DATA }}/floxhub_test_users.json"

    # Set the recording variable based on Justfile arguments
    export _FLOX_UNIT_TEST_RECORD=true
    if [ "{{ force }}" = "true" ]; then
        export _FLOX_UNIT_TEST_RECORD="force"
    else
        export _FLOX_UNIT_TEST_RECORD="missing"
    fi

    # Run the tests that will regenerate the mocks
    {{ cargo_test_invocation }} --no-fail-fast --filterset 'test(providers::publish) | test(commands::publish) | test(providers::catalog::tests::creates_new_catalog)'

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
    _FLOX_UNIT_TEST_RECORD={{ record }} {{ cargo_test_invocation }} {{ regex }}

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
    nix-unit package-builder/nef/tests \
        --arg nixpkgs-url "$COMMON_NIXPKGS_URL" \

test-buildenvLib:
    nix-unit buildenv/buildenvLib/tests

# Run the CLI unit tests, including impure tests
@impure-tests regex="": build
    {{ cargo_test_invocation }} {{ regex }} --features "extra-tests"

# Run the entire CLI test suite
test-cli: impure-tests integ-tests

# Run the test suite except for nix-plugins
@test-rust: impure-tests integ-tests nix-integ-tests

# Run the entire test suite, including impure unit tests
test-all: test-nix-plugins impure-tests integ-tests nix-integ-tests

# ---------------------------------------------------------------------------- #

# Refresh the prior-release lockfile fixtures under
#   test_data/manually_generated/prior_release_baselines/
# used by the cross-release tests: a current Flox release must still honor
# lockfiles produced by an earlier release (accept them without a re-lock, and
# re-lock them byte-for-byte).
#
# Usage:
#   just regen-prior-release-fixtures           # default pin (see below)
#   just regen-prior-release-fixtures 1.12.0    # explicit version
#
# Pin choice: the default is the EARLIEST release whose lockfiles the current
# release still reproduces byte-for-byte, NOT the previous minor. Pinning to
# this floor exercises the longest migration path (every schema migration from
# that release up to the current one must be a no-op), which is the widest
# scope we can test. Do not advance the pin as new releases ship.
#
# That floor is v1.12.0: the first release with the compose.composer
# schema-version drift fix (#4180). Earlier releases (1.10.x, 1.11.x) wrote a
# drifted composer the current release cannot accept as up-to-date or reproduce
# byte-for-byte, so composed (with_include) fixtures captured there would fail.
# Only raise the floor if a new, similarly incompatible boundary appears below
# the current floor.
#
# When to run:
#   - A test reports the current release no longer honors the floor fixtures
#     (investigate: likely a real serialization or predicate regression); or
#   - You are intentionally raising the floor.
#
# Requirements (so it cannot run in CI; a maintainer runs it):
#   1. Network access to fetch the release flake.
#   2. A local Nix store to build the rendered environment during activate.
#
# It deliberately uses 'nix build' rather than 'nix profile install': the
# former only populates the content-addressed store plus a temporary gcroot
# symlink, leaving your profile and PATH untouched. All prior-Flox state
# (HOME, XDG dirs, config) is redirected into a tempdir so the host is not
# touched either. The fixtures install no packages, so locking makes no catalog
# requests and needs no network beyond the release flake.
regen-prior-release-fixtures version="auto":
    #!/usr/bin/env bash
    set -euo pipefail

    BASELINES="test_data/manually_generated/prior_release_baselines"

    # --- Resolve the version pin to a release tag --------------------------
    # The default pin is the floor: the earliest release whose lockfiles the
    # current release still reproduces byte-for-byte (see the header comment).
    FLOOR_VERSION="1.12.0"
    VERSION="{{version}}"
    if [ "$VERSION" = "auto" ]; then
      VERSION="$FLOOR_VERSION"
    fi
    TAG="v${VERSION#v}"
    echo "==> Capturing prior-release fixtures with Flox $TAG"

    # Capture provenance now, before HOME is redirected away from ~/.gitconfig.
    CAPTURED_BY="$(git config user.email 2>/dev/null || echo unknown)"

    # --- Workspace (cleaned on exit), isolated from the host ---------------
    WORKROOT=$(mktemp -d)
    trap 'rm -rf "$WORKROOT"' EXIT

    # The 'nix develop' shell exports FLOX_* / NIX_PLUGINS / BUILDENV_BIN
    # overrides pointing at the in-tree (current-release) subsystems.  The
    # prior binary would pick these up and build with the new buildenv, then
    # fail to parse the result (e.g. "missing field `develop`").  Scrub them
    # so the prior binary uses its own compile-time-baked subsystems.
    while IFS='=' read -r name _; do
      case "$name" in
        FLOX_*|_FLOX_*|_flox_*) unset "$name" 2>/dev/null || true ;;
      esac
    done < <(env)
    unset NIX_PLUGINS BUILDENV_BIN _activate_d 2>/dev/null || true

    export HOME="$WORKROOT/home"
    export XDG_CACHE_HOME="$WORKROOT/cache"
    export XDG_DATA_HOME="$WORKROOT/data"
    export XDG_STATE_HOME="$WORKROOT/state"
    export XDG_CONFIG_HOME="$WORKROOT/config"
    export FLOX_CONFIG_DIR="$WORKROOT/config/flox"
    export FLOX_DISABLE_METRICS=true
    mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_DATA_HOME" \
             "$XDG_STATE_HOME" "$FLOX_CONFIG_DIR"

    # --- Build the prior release into the store (no profile/PATH changes) --
    nix build "github:flox/flox/$TAG" \
      --accept-flake-config \
      --out-link "$WORKROOT/flox-result"
    PRIOR_FLOX="$WORKROOT/flox-result/bin/flox"
    echo "==> Using $("$PRIOR_FLOX" --version)"

    # --- Lock one env dir with the prior release and copy the lockfile out --
    # $1 = throwaway env dir (already contains .flox), $2 = fixture out dir
    #
    # The fixtures install no packages, so locking makes zero catalog requests
    # and there is nothing to record. A package-bearing fixture would need its
    # catalog responses captured, which a Nix-store-built binary cannot do
    # (httpmock records to a read-only compile-time path); use a source-built
    # (cargo) binary then.
    capture() {
      local envdir="$1" outdir="$2"
      "$PRIOR_FLOX" activate --dir "$envdir" -c true
      cp "$envdir/.flox/env/manifest.lock" "$outdir/manifest.lock"
    }

    # --- plain shape -------------------------------------------------------
    PLAIN="$WORKROOT/plain"
    mkdir -p "$PLAIN"
    "$PRIOR_FLOX" init --dir "$PLAIN"
    cp "$BASELINES/plain/manifest.toml" "$PLAIN/.flox/env/manifest.toml"
    capture "$PLAIN" "$BASELINES/plain"

    # --- with_include shape (parent includes ../included) ------------------
    INC="$WORKROOT/with_include"
    mkdir -p "$INC/parent" "$INC/included"
    "$PRIOR_FLOX" init --dir "$INC/included"
    cp "$BASELINES/with_include/included/manifest.toml" \
       "$INC/included/.flox/env/manifest.toml"
    # Lock the included env so its manifest and lockfile are in sync;
    # otherwise the parent refuses to include it.
    "$PRIOR_FLOX" activate --dir "$INC/included" -c true
    "$PRIOR_FLOX" init --dir "$INC/parent"
    cp "$BASELINES/with_include/parent/manifest.toml" \
       "$INC/parent/.flox/env/manifest.toml"
    capture "$INC/parent" "$BASELINES/with_include/parent"

    # --- Record provenance in MANIFEST.json --------------------------------
    sha() { sha256sum "$1" | cut -d' ' -f1; }
    jq -n \
      --arg ver "${TAG#v}" \
      --arg on  "$(date -u +%Y-%m-%d)" \
      --arg by  "$CAPTURED_BY" \
      --arg pl  "$(sha "$BASELINES/plain/manifest.lock")" \
      --arg ql  "$(sha "$BASELINES/with_include/parent/manifest.lock")" \
      '{
        captured_with_flox_version: $ver,
        captured_on: $on,
        captured_by: $by,
        note: "Captured via '\''just regen-prior-release-fixtures'\''.",
        fixtures: {
          plain: { manifest_lock_sha256: $pl },
          with_include: {
            parent: { manifest_lock_sha256: $ql },
            included: { manifest_lock_sha256: null }
          }
        }
      }' > "$BASELINES/MANIFEST.json"

    # --- Clear the pending marker ------------------------------------------
    rm -f "$BASELINES/CAPTURE_PENDING"

    echo ""
    echo "==> Captured prior-release fixtures with $TAG."
    echo "    Next steps:"
    echo "      1. Inspect the diff under $BASELINES/"
    echo "      2. just unit-tests   &&   just integ-tests -- --filter prior-release"
    echo "      3. Commit the refreshed fixtures."


# ---------------------------------------------------------------------------- #

# Enters the Rust development environment
@work:
    # Note that this command is only really useful if you have
    # `just` installed outside of the `flox` environment already
    nix {{ nix_options }} develop

# ---------------------------------------------------------------------------- #

# Bump all flake dependencies and commit with a descriptive message
@bump-all:
    nix {{ nix_options }} flake update --commit-lock-file  \
         --commit-lockfile-summary "chore: flake bump";

# Bump a specific flake input and commit with a descriptive message
@bump input:
    nix {{ nix_options }} flake lock --update-input {{ input }}  \
         --commit-lock-file --commit-lockfile-summary         \
         "chore: bump '{{ input }}' flake input";

# ---------------------------------------------------------------------------- #

# Output licenses of all dependency crates
@license:
    cargo metadata --format-version 1              \
      |jq -r '.packages[]|[.name,.license]|@csv';

# ---------------------------------------------------------------------------- #

# Run a `flox` command
@flox +args="": build
    target/debug/flox {{ args }}

# Run a `flox` command using the catalog
@catalog-flox +args="": build
    echo "just: DEPRECATED TARGET: Use 'flox' instead" >&2;
    target/debug/flox {{ args }}

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
