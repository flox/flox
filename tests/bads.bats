#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# "The removal of tests which fail in CI will continue until
# test quality improves."
#
# ---------------------------------------------------------------------------- #

load test_support.bash;


# ---------------------------------------------------------------------------- #

## @test "flox generate config files in $FLOX_CONFIG_HOME" {
##   # The rust wrapper will not forward all commands to flox (bash)
##   # Help messages for instance are generated entirely by the argument parsing step,
##   # that precedes any command processing.
##   # As such this tests fails to see the "Updating ..." messages if used with `--help`.
##   # The first test forwarding to flox (subscribe, below) will and fails as well.
##   #
##   # This test will work until channels will be implemented in rust.
##   # At which point the messaging may change as well.
##   run "$FLOX_BIN" channels
##   assert_success
##   assert_output --partial "Updating \"$FLOX_CONFIG_HOME/gitconfig\""
##   skip "remaining portion of test depends on rust or bash execution"
##   assert_output --partial "Updating $FLOX_CONFIG_HOME/nix.conf"
## }


# ---------------------------------------------------------------------------- #

## @test "flox --prefix" {
##   run "$FLOX_BIN" --prefix
##   assert_success
##   assert_output "$FLOX_PACKAGE"
## }


# ---------------------------------------------------------------------------- #

## @test "flox --help" {
##   run $FLOX_BIN --help
##   assert_success
##   # the rust implementation generates its USAGE/help internally
##   if [ "$FLOX_IMPLEMENTATION" != "rust" ]; then
##     assert_output - <tests/usage.out
##   fi
## }


# ---------------------------------------------------------------------------- #

## @test "flox git remote -v" {
##   run $FLOX_BIN git remote -v
##   assert_success
##   assert_output - < /dev/null
## }


# ---------------------------------------------------------------------------- #

## # These next two tests are annoying:
## # - the `gh` tool requires GH_CONFIG_DIR
## # - while `nix` requires XDG_CONFIG_HOME
## #   - ... and because `nix` invokes `gh`, just provide them both
## @test "assert can log into github GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR" {
##   run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN gh auth status"
##   assert_success
##   assert_output --partial "✓ Logged in to github.com as"
## }
##
## @test "flox subscribe private with creds GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR" {
##   run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN subscribe flox-examples-private github:flox-examples/floxpkgs-private"
##   assert_success
##   assert_output --partial "subscribed channel 'flox-examples-private'"
## }


# ---------------------------------------------------------------------------- #

## # Keep environment in next test to prevent nix.conf rewrite warning.
## @test "flox unsubscribe private" {
##   run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN unsubscribe flox-examples-private"
##   assert_success
##   assert_output --partial "unsubscribed from channel 'flox-examples-private'"
## }


# ---------------------------------------------------------------------------- #

## # Again we need github connectivity for this.
## @test "flox push" {
##   run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN --debug push -e $TEST_ENVIRONMENT"
##   assert_success
##   assert_output --partial "To "
##   assert_output --regexp "\* \[new branch\] +origin/.*.$TEST_ENVIRONMENT -> .*.$TEST_ENVIRONMENT"
## }

## # ... and this.
## @test "flox pull" {
##   run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN pull -e $TEST_ENVIRONMENT"
##   assert_success
##   assert_output --partial "To "
##   assert_output --regexp "\* \[new branch\] +.*\.$TEST_ENVIRONMENT -> .*\.$TEST_ENVIRONMENT"
## }

# ---------------------------------------------------------------------------- #

## @test "flox list after flox pull should be exactly as before" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  2"
##   assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
##   ! assert_output --partial "stable.nixpkgs-flox.cowsay"
##   ! assert_output --partial "stable.nixpkgs-flox.dasel"
##   ! assert_output --partial "stable.nixpkgs-flox.jq"
## }


# ---------------------------------------------------------------------------- #

## @test "flox list after installing by store path should contain package" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  7"
##   assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
##   assert_output --partial "1  $HELLO_PACKAGE  $HELLO_PACKAGE_FIRST8"
## }


# ---------------------------------------------------------------------------- #

## @test "flox remove hello again" {
##   run $FLOX_BIN remove -e $TEST_ENVIRONMENT hello
##   assert_success
##   assert_output --partial "Removed 'hello' package(s) from '$TEST_ENVIRONMENT' environment."
## }


# ---------------------------------------------------------------------------- #

## @test "flox list after installing by nixpkgs flake should contain package" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  9"
##   assert_output --regexp "0  nixpkgs#hello +hello-"$VERSION_REGEX
##   assert_output --partial "1  $HELLO_PACKAGE  $HELLO_PACKAGE_FIRST8"
##   ! assert_output --partial "stable.nixpkgs-flox.hello"
## }


# ---------------------------------------------------------------------------- #

## @test "flox list after remove by nixpkgs flake 1 should not contain package" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  10"
##   assert_output --partial "0  $HELLO_PACKAGE  $HELLO_PACKAGE_FIRST8"
##   ! assert_output --partial "nixpkgs#hello"
##   ! assert_output --partial "stable.nixpkgs-flox.hello"
## }

## @test "flox rollback after flake removal 1" {
##   run $FLOX_BIN rollback -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 10 to 9."
## }


# ---------------------------------------------------------------------------- #

## @test "flox remove by nixpkgs flake 2" {
##   run $FLOX_BIN remove -e $TEST_ENVIRONMENT "flake:nixpkgs#legacyPackages.$NIX_SYSTEM.hello"
##   assert_success
##   assert_output --partial "Removed 'flake:nixpkgs#legacyPackages.$NIX_SYSTEM.hello' package(s) from '$TEST_ENVIRONMENT' environment."
## }


# ---------------------------------------------------------------------------- #

## @test "flox list to verify contents of generation 9 at generation 12" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  12"
##   assert_output --regexp "0  nixpkgs#hello +hello-"$VERSION_REGEX
##   assert_output --partial "1  $HELLO_PACKAGE  $HELLO_PACKAGE_FIRST8"
##   ! assert_output --partial "stable.nixpkgs-flox.hello"
## }


# ---------------------------------------------------------------------------- #

## @test "flox list after install should contain hello" {
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Curr Gen  2"
##   assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
## }


# ---------------------------------------------------------------------------- #

## @test "flox rollback to 1" {
##   run $FLOX_BIN rollback -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 2 to 1."
##   run $FLOX_BIN list -e $TEST_ENVIRONMENT
##   # generation 1 has no packages
##   assert_output --regexp ".*Packages"
## }


# ---------------------------------------------------------------------------- #

## @test "flox generations" {
##   run $FLOX_BIN generations -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "Generation 2:"
##   assert_output --partial "Path:"
##   assert_output --partial "Created:"
##   assert_output --partial "Last active:"
##   assert_output --partial "Log entries:"
##   assert_output --partial "installed stable.nixpkgs-flox.hello"
##   assert_output --partial "Generation 3:"
##   assert_output --partial "installed stable.nixpkgs-flox.cowsay stable.nixpkgs-flox.jq stable.nixpkgs-flox.dasel"
##   assert_output --partial "Generation 4:"
##   assert_output --partial "edited declarative profile (generation 4)"
##   assert_output --partial "Generation 5:"
##   assert_output --partial "edited declarative profile (generation 5)"
##   assert_output --partial "Generation 6:"
##   assert_output --partial "removed stable.nixpkgs-flox.hello"
## }


# ---------------------------------------------------------------------------- #

## @test "flox rollback to 0" {
##   run $FLOX_BIN rollback -e $TEST_ENVIRONMENT
##   assert_failure
##   assert_output --partial "ERROR: invalid generation '0'"
## }


# ---------------------------------------------------------------------------- #

## @test "flox rollback --to 2" {
##   run $FLOX_BIN switch-generation 2 -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --regexp "Switched environment '$TEST_ENVIRONMENT' from generation [0-9]+ to 2."
##   run $FLOX_BIN rollback --to 2 -e $TEST_ENVIRONMENT
##   assert_success
##   assert_output --partial "start and target generations are the same"
## }

# ---------------------------------------------------------------------------- #

##@test "flox publish" {
##  for key in AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY; do
##    if [ ! -v "$key" ]; then
##      skip "This test depends on $key but it is not set";
##    fi
##  done
##
##  # setup up temporary local channel
##  CHANNEL="$FLOX_TEST_HOME/channel"
##  run mkdir "$CHANNEL"
##  assert_success
##  run git -C "$CHANNEL" init
##  assert_success
##  run cp lib/templateFloxEnv/flake.nix "$CHANNEL"
##  assert_success
##  run git -C "$CHANNEL" add flake.nix
##  assert_success
##  run $FLOX_BIN flake update "$CHANNEL"
##  assert_success
##
##  run $FLOX_BIN --debug publish "github:flox/flox#flox-bash" \
##    --build-repo "git@github.com:flox/flox" \
##    --channel-repo "$CHANNEL" \
##    --upload-to 's3://flox-store-public?write-nar-listing=1&ls-compression=br'\
##    --download-from https://cache.floxdev.com
##    # TODO add --key-file or make content addressed
##  assert_success
##  assert_output --partial "flox publish completed"
##
##  CHANNEL_NAME=publish-test
##
##  run $FLOX_BIN subscribe "$CHANNEL_NAME" "$CHANNEL"
##  assert_success
##
##  run $FLOX_BIN search -c "$CHANNEL_NAME" flox
##  assert_success
##  assert_output "$CHANNEL_NAME.flox-bash"
##
##  run $FLOX_BIN unsubscribe "$CHANNEL_NAME"
##}


# ---------------------------------------------------------------------------- #

##@test "assert no access to private repository" {
##  # otherwise a cached version of the private repo may be used
##  run unlink $XDG_CACHE_HOME/nix
##  assert_success
##  run $FLOX_BIN flake metadata github:flox-examples/floxpkgs-private --no-eval-cache --no-write-lock-file --json
##  assert_failure
##  run ln -s ~/.cache/nix $XDG_CACHE_HOME/nix
##  assert_success
##}

##@test "flox subscribe private without creds" {
##  run $FLOX_BIN subscribe flox-examples-private github:flox-examples/floxpkgs-private
##  assert_failure
##  assert_output --partial 'ERROR: could not verify channel URL: "github:flox-examples/floxpkgs-private"'
##}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
