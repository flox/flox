#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox (no subcommand) command
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# ---------------------------------------------------------------------------- #

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/test"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup_file() {
  common_file_setup
}

teardown_file() {
  common_file_teardown
}

setup() {
  common_test_setup
  project_setup
}
teardown() {
  common_test_teardown
  project_teardown
}

@test "f1: simplify flox 'no command' info" {
  # There are three variations of this message but for simplicity we only test
  # with at least one inactivate environment.
  "$FLOX_BIN" init

  run "$FLOX_BIN"
  assert_success
  # Specific version is tested in `version.bats`
  assert_output --partial "flox version "
  assert_output --partial - << 'EOF'
Usage: flox OPTIONS (init|activate|search|install|...) [--help]

Use 'flox --help' for full list of commands and more information

No active environments. Use 'flox envs' to list all environments.
EOF
}

@test "f?: 'flox --help' has 0 exit code" {
  run "$FLOX_BIN" --help
  assert_success
}

@test "f2: commands are grouped by action and ordered by use" {
  run "$FLOX_BIN" --help

  assert_output - << 'EOF'
Flox is a virtual environment and package manager all in one.

With Flox you create environments that layer and replace dependencies just where
it matters, making them portable across the full software lifecycle.

Usage: flox [[-v]... | -q] [-V] [COMMAND ...]

Manage environments
    init           Create an environment in the current directory
    envs           Show active and available environments
    delete         Delete an environment

Use environments
    activate       Enter the environment, type 'exit' to leave
    services       Interact with services

Discover packages
    search         Search for system or library packages to install
    show           Show details about a single package

Modify environments
    install, i     Install packages into an environment
    list, l        List packages installed in an environment
    edit           Edit declarative environment configuration file
    include        Compose environments together
    upgrade        Upgrade packages in an environment
    uninstall      Uninstall installed packages from an environment
    generations    Version control for environments pushed to FloxHub

Share with others
    build          Build packages for Flox
    publish        Publish packages for Flox
    push           Send an environment to FloxHub
    pull           Pull an environment from FloxHub
    containerize   Containerize an environment

Administration
    auth           FloxHub authentication commands
    config         View and set configuration options
    gc             Garbage collects any data for deleted environments.

Available options:
    -v, --verbose  Increase logging verbosity
                   Invoke multiple times for increasing detail.
    -q, --quiet    Silence logs except for errors
    -V, --version  Print the version of the program
    -h, --help     Prints help information

Run 'man flox' for more details.
EOF
}
