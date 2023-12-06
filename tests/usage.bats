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


@test "f2: command grouping changes 1: 'Local Development Commands' listed in order" {
    run --separate-stderr "$FLOX_CLI" --help;
    assert_line -n 4 --regexp '^Local Development Commands';
    assert_line -n 5 --regexp '^    init[ ]+[\w .,]+';
    assert_line -n 6 --regexp '^    activate[ ]+[\w .,]+';
    assert_line -n 7 --regexp '^    search[ ]+[\w .,]+';
    assert_line -n 8 --regexp '^    show[ ]+[\w .,]+';
    assert_line -n 9 --regexp '^    install[ ]+[\w .,]+';
    assert_line -n 10 --regexp '^    uninstall[ ]+[\w .,]+';
    assert_line -n 11 --regexp '^    edit[ ]+[\w .,]+';
    assert_line -n 12 --regexp '^    list[ ]+[\w .,]+';
    assert_line -n 13 --regexp '^    delete[ ]+[\w .,]+';
}

@test "f3: command grouping changes 2: 'Sharing Commands' listed in order" {
    run "$FLOX_CLI" --help
    assert_line -n 14 --regexp '^Sharing Commands';
    assert_line -n 15 --regexp '^    push[ ]+[\w .,]+';
    assert_line -n 16 --regexp '^    pull[ ]+[\w .,]+';
    assert_line -n 17 --regexp '^    containerize[ ]+[\w .,]+';
}

@test "f5: command grouping changes 3: move lesser used or not polished commands to 'Additional Commands' section with help tip." {
    run "$FLOX_CLI" --help
    assert_output --partial - << EOF
Additional Commands. Use "flox COMMAND --help" for more info
    upgrade, config, wipe-history, history
EOF
}

@test "f6: remove stability from flox --help command: Only show stability for commands that support it" {
    run "$FLOX_CLI" --help
    refute_output --partial "--stability"
}

@test "f7: remove debug: don't show debug in flox and only show in flox --help {
    skip "Unclear"
}
