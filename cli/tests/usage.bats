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

setup_file() {
  common_file_setup
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown_file() {
  unset _FLOX_USE_CATALOG_MOCK
  common_file_teardown
}

setup() {
  common_test_setup
}
teardown() {
  common_test_teardown
}

@test "f1: simplify flox 'no command' info" {
  run "$FLOX_BIN"
  assert_success
  # Specific version is tested in `version.bats`
  assert_output --partial "flox version "
  assert_output --partial - << 'EOF'
Usage: flox OPTIONS (init|activate|search|install|...) [--help]

Use 'flox --help' for full list of commands and more information

First time? Create an environment with 'flox init'
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

Local Development Commands
    init           Create an environment in the current directory
    activate       Enter the environment, type 'exit' to leave
    search         Search for system or library packages to install
    show           Show details about a single package
    install, i     Install packages into an environment
    uninstall      Uninstall installed packages from an environment
    edit           Edit declarative environment configuration file
    list, l        List packages installed in an environment
    delete         Delete an environment
    services       Interact with services

Sharing Commands
    push           Send an environment to FloxHub
    pull           Pull an environment from FloxHub
    containerize   Containerize an environment

Additional Commands. Use "flox COMMAND --help" for more info
    auth, config, envs, gc, include, upgrade 

Available options:
    -v, --verbose  Increase logging verbosity
                   Invoke multiple times for increasing detail.
    -q, --quiet    Silence logs except for errors
    -V, --version  Print the version of the program
    -h, --help     Prints help information

Run 'man flox' for more details.
EOF
}
