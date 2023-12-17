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
PKGDB_BIN := "${PWD}/build/pkgdb/bin/pkgdb"
FLOX_BIN := "${PWD}/build/cli/target/debug/flox"
cargo_test_invocation := "PKGDB_BIN=${PKGDB_BIN} cargo test --workspace"
vscode_cpp_config := "./.vscode/c_cpp_properties.json"


# ---------------------------------------------------------------------------- #

_default:
    @just --list --unsorted


# ---------------------------------------------------------------------------- #

# (Re)initialize Makefile.in templates.
# This is necessary if you change a `Makefile.am' or `configure.ac' file.
bootstrap:
    @./bootstrap.sh


# ---------------------------------------------------------------------------- #

# Prepare the build area and lock configuration options.
configure *args='':
    @mkdir -p build;
    @pushd build;
    @../configure --prefix="$PWD/out" "$@"


# ---------------------------------------------------------------------------- #

# Print the paths of all of the binaries
bins:
    @echo "{{PKGDB_BIN}}"
    @echo "{{FLOX_BIN}}"

# ---------------------------------------------------------------------------- #

# Build only pkgdb
build-pkgdb *args='':
    @make -C build -j pkgdb "$@"

# Build only flox
build-cli *args='':
    @make -C build -j cli "$@"

# Build the binaries
build *args='':
    @make -C build -j "$@"


# ---------------------------------------------------------------------------- #

# Run the pkgdb tests
test-pkgdb: build-pkgdb
    @make -C build/pkgdb -f Makefile.bak -j tests;
    @make -C build/pkgdb -f Makefile.bak check;

# Run the end-to-end test suite
functional-tests +bats_args="": build
    @flox-tests --pkgdb "{{PKGDB_BIN}}" --flox "{{FLOX_BIN}}" {{bats_args}}

# Run the CLI integration test suite
integ-tests +bats_args="": build
    @flox-cli-tests --pkgdb "{{PKGDB_BIN}}" \
     --flox "{{FLOX_BIN}}" -- {{bats_args}}

# Run a specific CLI integration test file by name (not path)
integ-file +bats_args="": build
    @flox-cli-tests --pkgdb "{{PKGDB_BIN}}" \
     --flox "{{FLOX_BIN}}" -- {{bats_args}}

# Run the CLI unit tests
unit-tests regex="": build
    @pushd cli;                            \
     {{cargo_test_invocation}} {{regex}};  \
     popd;

# Run the CLI unit tests, including impure tests
impure-tests regex="": build
    @pushd cli;                                                     \
     {{cargo_test_invocation}} {{regex}} --features "extra-tests";  \
     popd;

# Run the entire CLI test suite
test-cli: impure-tests integ-tests functional-tests

# Run the entire test suite, including impure unit tests
test-all: test-pkgdb impure-tests integ-tests functional-tests 


# ---------------------------------------------------------------------------- #

# Enters the Rust development environment
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
#
#
#
# ============================================================================ #
