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
cargo_test_invocation := "PKGDB_BIN=${PKGDB_BIN} cargo test --workspace"
vscode_cpp_config := "./.vscode/c_cpp_properties.json"


# ---------------------------------------------------------------------------- #

_default:
    @just --list --unsorted


# ---------------------------------------------------------------------------- #


# Print the paths of all of the binaries
bins:
    @echo "{{PKGDB_BIN}}"
    @echo "{{FLOX_BIN}}"


# ---------------------------------------------------------------------------- #


# Build only pkgdb
build-pkgdb:
    @make -C pkgdb -j;

# Build only flox
build-cli: build-pkgdb
    @pushd cli; cargo build -q; popd

build-docs:
  @echo "TODO";

build-all: build-pkgdb build-cli build-docs


# ---------------------------------------------------------------------------- #

# Run the pkgdb tests
test-pkgdb: build-pkgdb
    @make -C pkgdb tests;
    @make -C pkgdb check;

# Run the CLI unit tests
test-cli-unit regex="": build-cli
    @pushd cli;                            \
     {{cargo_test_invocation}} {{regex}};  \
     popd;


# Run the CLI unit tests, including impure tests
test-cli-impure regex="": build-cli
    @pushd cli;                                                     \
     {{cargo_test_invocation}} {{regex}} --features "extra-tests";  \
     popd;

# Run the integration test suite
test-cli-integration +bats_args="": build-cli
    @flox-cli-tests                     \
        --pkgdb "{{PKGDB_BIN}}"         \
        --flox "{{FLOX_BIN}}"           \
        -- {{bats_args}};

# Run all of the cli tests
test-cli: build-cli test-cli-unit test-cli-impure test-cli-integration

# Run end2end tests
test-end2end +args="": build-cli
    @pytest \
      --emoji \
      --durations=0 \
      --capture=no \
      -v \
      {{args}};

# Run all tests
test-all: test-pkgdb test-cli test-end2end


# ---------------------------------------------------------------------------- #


run-pkgdb +args="": build-pkgdb
  @./pkgdb/bin/pkgdb {{args}};

run-cli +args="": build-cli
  @./cli/target/debug/flox {{args}};

run-end2end: test-end2end


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

clean-end2end:
    @rm "${PWD}/.pytest_cache" -rf;

clean-docs:
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

# Configure VS Code's C++ environment
config-vscode:
    @pushd pkgdb; make -j -s cdb; popd
    @if [ ! -f {{vscode_cpp_config}} ]; \
        then echo "{}" > {{vscode_cpp_config}}; \
        fi
    @echo $(jq '.configurations.cppStandard = "c++20"' {{vscode_cpp_config}}) \
        > {{vscode_cpp_config}};
    @echo $(jq \
        '.configurations.compileCommands = \
        "${workspaceFolder}/pkgdb/compile_commands.json"' \
        {{vscode_cpp_config}}) > {{vscode_cpp_config}}


# ---------------------------------------------------------------------------- #
# Deprecated recipes


build:
  @echo "Deprecated: Please use \`just build-all\` recipe!";
  @just build-all;

functional-tests +args="":
  @echo "Deprecated: Please use \`just build-all\` recipe!";
  @just build-all {{args}};

integ-tests +args="":
  @echo "Deprecated: Please use \`just test-cli-integration\` recipe!";
  @just test-cli-integration {{args}};

integ-file +args="":
  @echo "Deprecated: Please use \`just test-cli-integration\` recipe!";
  @just test-cli-integration {{args}};

unit-tests args="":
  @echo "Deprecated: Please use \`just test-cli-unit\` recipe!";
  @just test-cli-unit {{args}};

impure-tests args="": build
  @echo "Deprecated: Please use \`just test-cli-impure\` recipe!";
  @just test-cli-impure {{args}};


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
