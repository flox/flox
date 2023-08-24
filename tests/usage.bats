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

setup() {
    common_test_setup
}
teardown() {
    common_test_teardown
}

@test "f1: simplify flox 'no command' info" {
    run "$FLOX_CLI"
    assert_success
    # FLOX_VERSION is set by the `flox run` command
    # and thus deviates from the expected version.
    assert_output --regexp - << EOF
flox version \d+.\d+.\d+-.+

Usage: flox OPTIONS \(init|activate|search|install|\.\.\.\) \[--help\]

Use "flox --help" for full list of commands and more information

First time\? Create an environment with "flox init"
EOF
}

@test "f?: 'flox --help' has 0 exit code" {
    run "$FLOX_CLI"
    assert_success
}


@test "f2: command grouping changes 1: Add 'Local Development Commands' and list in order" {
    run "$FLOX_CLI" --help
    assert_output --partial - << EOF
Local Development Commands
    init           Create an environment in the current directory
    activate       Activate environment
    search         Search packages in subscribed channels
    install        Install a package into an environment
    uninstall      Uninstall installed packages from an environment
    edit           Edit declarative environment configuration
    run            Run app from current project
    list           List (status?) packages installed in an environment
    nix            Access to the nix CLI
    delete         Delete an environment
EOF
}

@test "f3: command grouping changes 2: introduce "Sharing Commands" and include 1/ push 2/ pull 3/ containerize" {
    run "$FLOX_CLI" --help
    assert_output --partial - << EOF
Sharing Commands
    push           Send environment to flox hub
    pull           Pull environment from flox hub
    containerize   Containerize an environment
EOF
}

@test "f5: command grouping changes 3: move lesser used or not polished commands to 'Additional Commands' section with help tip." {
    run "$FLOX_CLI" --help
    assert_output --partial - << EOF
Additional Commands. Use "flox COMMAND --help" for more info
    build, upgrade, import, export, config, wipe-history, subscribe, unsubscribe,
    channels, history, print-dev-env, shell
EOF
}

@test "f6: remove stability from flox --help command: Only show stability for commands that support it" {
    run "$FLOX_CLI" --help
    refute_output --partial "--stability"
}

@test "f7: remove debug: don't show debug in flox and only show in flox --help {
    skip "Unclear"
}
