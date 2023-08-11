#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test `flox publish` command
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

    export HELLO_VERSION="$($FLOX_CLI nix eval --raw nixpkgs-flox#hello.version)"
}

# Giving each test an individual channel to allow parallel runs.
# The setup will create a channel repo with a `hello` package.
# `flox publish` resolves local repositories to their upstream counterpart.
# Since we don't want to manage and pollute upstream repositories,
# we set the repository as its own remote, and can verify publishes by checking
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
    export FLOX_SIGNING_KEY="$(mktemp)"
    $FLOX_CLI nix key generate-secret --key-name "test" >"$FLOX_SIGNING_KEY"
}

# Given a valid pacakge, a signing key and a binary cache,
# flox publish is expected to succeed.
@test "flox publish" {
    run $FLOX_CLI -v publish "$CHANNEL#hello"
    assert_success

    local EXPECTED_PATH='catalog/hello/$HELLO_VERSION-*'
    run git -C "$CHANNEL" ls-tree "catalog/$NIX_SYSTEM" "$EXPECTED_PATH"
    assert_success
}

# Publish requires a signing key.
# Without a key, flox will fail with a meaningful error.
@test "flox publish fails without signing-key" {
    unset FLOX_SIGNING_KEY

    run $FLOX_CLI -v publish "$CHANNEL#hello"
    assert_failure
    assert_output --partial "Signing key is required!"
}

# Publish requires a cache url.
# Without a cache url, flox will fail with a meaningful error.
@test "flox publish fails without cache url" {
    unset FLOX_CACHE_URL

    run $FLOX_CLI -v publish "$CHANNEL#hello"
    assert_failure
    assert_output --partial "Cache url is required!"
}

teardown_file() {
    kill "$NIX_SERVE_PID"
}
