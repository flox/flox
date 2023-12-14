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
cargo_test_invocation := "cargo test --workspace"


# ---------------------------------------------------------------------------- #

_default:
    @just --list --unsorted


# ---------------------------------------------------------------------------- #

build-pkgdb:
    @make -C pkgdb -j;

build-cli: build-pkgdb
    @pushd cli; cargo build; popd

build-docs:
  @echo "TODO";

build-all: build-pkgd build-cli build-docs


# ---------------------------------------------------------------------------- #

test-pkgdb: build-pkgdb
    @make -C pkgdb -j tests;
    @make -C pkgdb check;

# Run the CLI unit tests
test-cli-unit regex="": build
    @pushd cli;                            \
     {{cargo_test_invocation}} {{regex}};  \
     popd;

# Run the test suite, including impure tests
test-cli-impure regex="": build
    @pushd cli;                                                     \
     {{cargo_test_invocation}} {{regex}} --features "extra-tests";  \
     popd;

# Run the integration test suite
test-cli-integration: build
    @flox-cli-tests --pkgdb "${PWD}/pkgdb/bin/pkgdb"        \
                    --flox "${PWD}/cli/target/debug/flox";

# Run all of the cli tests
test-cli: build test-cli-unit test-cli-impure test-cli-integration

# Run end2end tests
test-end2end +args="": build
    @pytest \
      --emoji \
      --durations=0 \
      --capture=no \
      -v \
      {{args}};

# Run all tests
test-all: test-pkgdb functional-tests integ-tests


# ---------------------------------------------------------------------------- #


run-pkgdb +args="": build-pkgdb
  @./pkgdb/bin/pkgdb {{args}};

run-cli +args="": build-cli
  @./cli/target/debug/flox {{args}};


# ---------------------------------------------------------------------------- #

fmt-pkgdb:
    @make -C pkgdb fmt;

fmt-cli:
    @pre-commit run rustfmt "${PWD}/cli";
    @pre-commit run clippy "${PWD}/cli";

fmt-nix:
    @pre-commit run alejandra "${PWD}/flake.nix" "${PWD}/pkgs";

fmt-end2end:
    @pre-commit run ruff --files "${PWD}/end2end";

fmt-docs:
  @echo "TODO";

fmt-all: fmt-pkgdb fmt-cli fmt-nix fmt-end2end fmt-docs

# ---------------------------------------------------------------------------- #


clean-pkgdb:
    @make -C pkgdb clean;

clean-cli:
    @pushd cli; cargo clean; popd;

clean-end2end
    @rm "${PWD}/.pytest_cache" -rf;

claen-docs:
  @echo "TODO";

clean-all: clean-pkgdb clean-cli clean-end2end clean-docs


# ---------------------------------------------------------------------------- #

# Enters the development environment
work:
    @# Note that this command is only really useful if you have
    @# `just` installed outside of the `flox` environment already
    @nix {{nix_options}} develop


# ---------------------------------------------------------------------------- #

# Bump all flake dependencies and commit with a descriptive message
bump-all:
    @nix {{nix_options}} flake update --commit-lock-file  \
         --commit-lockfile-summary "chore: flake bump";

# Bump a specific flake input and commit with a descriptive message
bump input:
    @nix {{nix_options}} flake lock --update-input {{input}}  \
         --commit-lock-file --commit-lockfile-summary         \
         "chore: bump '{{input}}' flake input";


# ---------------------------------------------------------------------------- #

# Output licenses of all dependency crates
license:
    @pushd cli;                                     \
     cargo metadata --format-version 1              \
       |jq -r '.packages[]|[.name,.license]|@csv';


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
