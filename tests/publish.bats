#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox publish2` command
#
# This subcommand requires the configuration of a `cache_url`, i.e. a store to
# copy binaries to, as well as a `sign_key` file to enfore signing binaries.
# Both values can either be given via the command line as
#     --cache-url <url>     and
#     --sign-key <path>
# or via the config subsystem.
# Here, we use the config subsystem though the `FLOX_CACHE_URL`
# and `FLOX_SIGN_KEY`env variables.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# * Setup a nix substituter and configure flox to use it
# * Get version and output hash for the `hello` package
#   to check the generated catalog against.
setup_file() {
    nix-serve -p 8081 &
    export NIX_SERVE_PID="$!"

    # Set the `cache_url` config value
    export FLOX_CACHE_URL="http://localhost:8081"

    HELLO_OUT_PATH="$($FLOX_CLI nix eval --raw nixpkgs-flox#hello)"
    export HELLO_HASH_FIRST_8="${HELLO_OUT_PATH:11:8}" # skip /nix/store, take 8
    export HELLO_VERSION="$($FLOX_CLI nix eval --raw nixpkgs-flox#hello.version)"
}

# Giving each test an individual channel to allow parallel runs.
# The setup will create a channel repo with a `hello` package.
# `flox publish2` resolves local repositories to their upstream counterpart.
# Since we don't want to manage and pollute upstream repositories,
# we set the reposotry as its own remote, and can verify publishes by checking
# out the respective catalog branch.
# Note: this is not the intended production use as it causes system dependent
#       snapshots, but it is a minimally invasive solution for testing.
setup() {
    # setup up temporary local channel
    export CHANNEL="$(mktemp -d)"
    cp flox-bash/lib/templateFloxEnv/flake.nix "$CHANNEL/flake.nix"
    mkdir -p "$CHANNEL/pkgs/hello"
    echo '{hello}: hello' >>"$CHANNEL/pkgs/hello/default.nix"

    # put channel under version control
    git -C "$CHANNEL" init
    git -C "$CHANNEL" add flake.nix pkgs/hello/default.nix
    $FLOX_CLI flake update "$CHANNEL"
    git -C "$CHANNEL" add flake.lock
    git -C "$CHANNEL" commit -m "root commit"

    # set remote to the local repository to minimize external state
    git -C "$CHANNEL" remote add origin "$CHANNEL"
    git -C "$CHANNEL" fetch
    git -C "$CHANNEL" branch --set-upstream-to="origin/$(git -C "$CHANNEL" branch --show-current)"

    # Set the `sign_key` config value
    export FLOX_SIGN_KEY="$(mktemp)"
    $FLOX_CLI nix key generate-secret --key-name "test" >"$FLOX_SIGN_KEY"
}

# Given a valid pacakge, a signing key and a binary cache,
# flox publish2 is expected to succeed.
@test "flox publish2" {
    run $FLOX_CLI -v publish2 "$CHANNEL#hello"
    assert_success

    local EXPECTED_PATH=catalog/hello/$HELLO_VERSION-$HELLO_HASH_FIRST_8.json
    run git -C "$CHANNEL" show "catalog/$NIX_SYSTEM:$EXPECTED_PATH"
    assert_success
}

teardown_file() {
    kill "$NIX_SERVE_PID"
}
