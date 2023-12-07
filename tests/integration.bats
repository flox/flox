#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# flox CLI tests run in two contexts:
# - unit tests only to be run from within package build
# - unit and integration tests to be run from command line
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=integration
#

# ---------------------------------------------------------------------------- #

setup_file() {
  skip "Skipping --bash-passthru tests";
  common_file_setup;
  require_expect;
  hello_pkg_setup;
  # We can't really parallelize these because we depend on past test states.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true;
}


# ---------------------------------------------------------------------------- #

@test "assert testing home $FLOX_TEST_HOME" {
  run sh -c "test -d $FLOX_TEST_HOME"
  assert_success
}

@test "flox eval" {

  skip "DEPRECATED"

  # Evaluate a Nix expression given on the command line:
  run $FLOX_BIN eval --expr '1 + 2'
  assert_success
  assert_output --partial 3

  # Evaluate a Nix expression to JSON:
  run $FLOX_BIN eval --json --expr '{ x = 1; }'
  assert_success
  echo '{"x":1}' | assert_output -

  # Evaluate a Nix expression from a file:
  run $FLOX_BIN eval -f ./tests tests.name
  assert_success
  echo '"tests-1.2.3"' | assert_output -

  # Get the current version of the nixpkgs flake:
  run $FLOX_BIN eval --raw 'nixpkgs#lib.version'
  assert_success
  # something like "23.05pre-git"
  assert_output --regexp "[0-9][0-9].[0-9][0-9]"

  # Print the store path of the Hello package:
  run $FLOX_BIN eval --raw nixpkgs#hello
  assert_success
  assert_output --regexp "/nix/store/.*-hello-"

  # Get a list of checks in the nix flake:
  run $FLOX_BIN eval github:nixos/nix#checks.x86_64-linux --apply builtins.attrNames
  assert_success
  # Unfortunately we need to do a partial match because our attempt
  # to override the nixpkgs input throws a warning on a non-capacitated
  # flake.
  assert_output --partial '[ "binaryTarball" "dockerImage" "installTests" "nixpkgsLibTests" "perlBindings" ]'

  # Generate a directory with the specified contents:
  run $FLOX_BIN eval --write-to ./tests/out --expr '{ foo = "bar"; subdir.bla = "123"; }'
  assert_success
  run cat ./tests/out/foo
  assert_success
  echo bar | assert_output -
  run cat ./tests/out/subdir/bla
  assert_success
  echo 123 | assert_output -
  rm -f ./tests/out/foo ./tests/out/subdir/bla
  rmdir ./tests/out/subdir ./tests/out
}

@test "flox subscribe public" {
  run $FLOX_BIN subscribe flox-examples github:flox-examples/floxpkgs
  assert_success
  assert_output --partial "subscribed channel 'flox-examples'"
}

@test "flox unsubscribe public" {
  run $FLOX_BIN unsubscribe flox-examples
  assert_success
  assert_output --partial "unsubscribed from channel 'flox-examples'"
}

@test "flox auth2 login" {
  run $FLOX_BIN auth2 login
  assert_output --partial "Please visit https://github.com/login/device in your browser"
  assert_failure
}

@test "assert not logged into github" {
  run $FLOX_BIN gh auth status
  assert_failure
  assert_output --partial "You are not logged into any GitHub hosts. Run gh auth login to authenticate."
}

@test "flox create -e $TEST_ENVIRONMENT" {
  run $FLOX_BIN --bash-passthru create -e "$TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "created environment $TEST_ENVIRONMENT ($NIX_SYSTEM)"
}

@test "flox create -e $TEST_ENVIRONMENT fails when run again" {
  run $FLOX_BIN --bash-passthru create -e "$TEST_ENVIRONMENT"
  assert_failure
  assert_output --partial "ERROR: environment $TEST_ENVIRONMENT ($NIX_SYSTEM) already exists"
}

@test "flox install hello" {
  run $FLOX_BIN --bash-passthru install -e "$TEST_ENVIRONMENT" hello
  assert_success
  assert_output --partial "Installed 'hello' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox export -> import is a no-op" {
  run sh -c "$FLOX_BIN --bash-passthru export -e $TEST_ENVIRONMENT | $FLOX_BIN --bash-passthru --debug import -e $TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "No environment changes detected"
}

@test "flox install nixpkgs-flox.hello" {
  run $FLOX_BIN --bash-passthru install -e $TEST_ENVIRONMENT nixpkgs-flox.hello
  assert_success
  assert_output --partial "No change! Package(s) 'nixpkgs-flox.hello' already installed into '$TEST_ENVIRONMENT' environment."
}

@test "flox install stable.nixpkgs-flox.hello" {
  run $FLOX_BIN --bash-passthru install -e $TEST_ENVIRONMENT stable.nixpkgs-flox.hello
  assert_success
  assert_output --partial "No change! Package(s) 'stable.nixpkgs-flox.hello' already installed into '$TEST_ENVIRONMENT' environment."
}

# A rose by any other name ...
@test "flox subscribe nixpkgs-flox-dup" {
  run $FLOX_BIN subscribe nixpkgs-flox-dup github:flox/nixpkgs-flox/master
  assert_success
  assert_output --partial "subscribed channel 'nixpkgs-flox-dup'"
}

@test "flox install stable.nixpkgs-flox-dup.hello" {
  run $FLOX_BIN --bash-passthru install -e $TEST_ENVIRONMENT stable.nixpkgs-flox-dup.hello
  assert_failure
  assert_output --regexp ".*error: package nixpkgs-flox.$NIX_SYSTEM.stable.hello.latest is identical to package nixpkgs-flox-dup.$NIX_SYSTEM.stable.hello.latest"
}

@test "flox install cowsay jq dasel" {
  run $FLOX_BIN  --bash-passthru --debug install -e $TEST_ENVIRONMENT cowsay jq dasel
  assert_success
  assert_output --partial "created generation 3"
}

@test "flox list after install should contain cowsay and hello" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox edit remove hello" {
  EDITOR=./tests/remove-hello run $FLOX_BIN --bash-passthru edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified."
}

@test "verify flox edit removed hello from manifest.json" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "verify flox edit removed hello from flox.nix" {
  EDITOR=cat run $FLOX_BIN --bash-passthru edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial 'nixpkgs-flox.cowsay = {'
  assert_output --partial 'nixpkgs-flox.dasel = {'
  ! assert_output --partial 'nixpkgs-flox.hello = {'
  assert_output --partial 'nixpkgs-flox.jq = {'
  ! assert_output --partial "created generation"
}

@test "flox edit add hello" {
  EDITOR=./tests/add-hello run $FLOX_BIN --bash-passthru edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified."
}

@test "verify flox edit added hello to manifest.json" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "verify flox edit added hello to flox.nix" {
  EDITOR=cat run $FLOX_BIN --bash-passthru edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial 'nixpkgs-flox.cowsay = {'
  assert_output --partial 'nixpkgs-flox.dasel = {'
  assert_output --partial 'nixpkgs-flox.hello = {'
  assert_output --partial 'nixpkgs-flox.jq = {'
  ! assert_output --partial "created generation"
}

@test "flox edit preserves comments" {
  EDIT_ENVIRONMENT=_edit_testing_
  run $FLOX_BIN --bash-passthru create -e "$EDIT_ENVIRONMENT"
  assert_success

  EDITOR=./tests/add-comment run $FLOX_BIN --bash-passthru edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial "Environment '$EDIT_ENVIRONMENT' modified."

  EDITOR=cat run $FLOX_BIN --bash-passthru edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial "# test comment"

  run $FLOX_BIN --bash-passthru delete --force -e "$EDIT_ENVIRONMENT"
  assert_success
}

@test "flox remove hello" {
  run $FLOX_BIN --bash-passthru remove -e $TEST_ENVIRONMENT hello
  assert_success
  assert_output --partial "Removed 'hello' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox list after remove should not contain hello" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox list of generation 3 should contain hello" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT 3
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox history should contain the install and removal of stable.nixpkgs-flox.hello" {
  run $FLOX_BIN --bash-passthru history -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "removed stable.nixpkgs-flox.hello"
  assert_output --partial "installed stable.nixpkgs-flox.cowsay stable.nixpkgs-flox.jq stable.nixpkgs-flox.dasel"
  assert_output --partial "installed stable.nixpkgs-flox.hello"
  assert_output --partial "created environment"
}

@test "flox remove from nonexistent environment should fail" {
  run $FLOX_BIN --bash-passthru remove -e does-not-exist hello
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_BIN git branch -a | grep -q does-not-exist"
  assert_failure
  # -- output differs --
  # expected (0 lines):
  #
  # actual (2 lines):
  #   Updating "/tmp/tmp.KrigRID1eZ/.config/flox/gitconfig"
  #   Updating /tmp/tmp.KrigRID1eZ/.config/flox/gitconfig
  #assert_output - < /dev/null
}

@test "flox remove channel package by index" {
  TEST_CASE_ENVIRONMENT="$(echo $RANDOM | md5sum | head -c 20; echo)"

  run $FLOX_BIN --bash-passthru install -e "$TEST_CASE_ENVIRONMENT" hello
  assert_success

  run $FLOX_BIN --bash-passthru list -e "$TEST_CASE_ENVIRONMENT"
  assert_success
  assert_output --regexp "0 +stable.nixpkgs-flox.hello +$VERSION_REGEX"

  run $FLOX_BIN --bash-passthru remove -e "$TEST_CASE_ENVIRONMENT" 0
  assert_success
  assert_output --partial                                                \
    "Removed '0' package(s) from '$TEST_CASE_ENVIRONMENT' environment."

  run $FLOX_BIN --bash-passthru list -e "$TEST_CASE_ENVIRONMENT"
  assert_success
  refute_output --partial "stable.nixpkgs-flox.hello"

  # teardown
  run $FLOX_BIN --bash-passthru delete -e "$TEST_CASE_ENVIRONMENT" -f
  assert_success
}

@test "flox remove flake package by index" {
  TEST_CASE_ENVIRONMENT="$(echo $RANDOM | md5sum | head -c 20; echo)"

  run $FLOX_BIN --bash-passthru install -e "$TEST_CASE_ENVIRONMENT" nixpkgs#hello
  assert_success

  run $FLOX_BIN --bash-passthru list -e "$TEST_CASE_ENVIRONMENT"
  assert_success
  assert_output --regexp  \
    "0 +nixpkgs#legacyPackages\.$NIX_SYSTEM\.hello +$VERSION_REGEX"

  run $FLOX_BIN --bash-passthru remove -e "$TEST_CASE_ENVIRONMENT" 0
  assert_success
  assert_output --partial  \
    "Removed '0' package(s) from '$TEST_CASE_ENVIRONMENT' environment."

  run $FLOX_BIN --bash-passthru list -e "$TEST_CASE_ENVIRONMENT"
  assert_success
  refute_output --partial "nixpkgs#legacyPackages.$NIX_SYSTEM.hello"

  # teardown
  run $FLOX_BIN --bash-passthru delete -e "$TEST_CASE_ENVIRONMENT" -f
  assert_success
}

# To generate the test cases in tests/upgrade, use the following commands:
# flox subscribe nixpkgs-flox-upgrade-test github:flox/nixpkgs-flox/e4327de84f3aa8417e332a864c3b58c83b44832b
# flox install nixpkgs-flox-upgrade-test.curl -e _upgrade_test_
# flox install nixpkgs-flox-upgrade-test.ripgrep -e _upgrade_test_
# flox export -e _upgrade_test_ > upgrade.tar
# mkdir tests/upgrade/$system
# tar -xvf upgrade.tar --exclude flake.nix --exclude flake.lock -C tests/upgrade/$system
# ln -s ../../../../lib/templateFloxEnv/flake.lock tests/upgrade/$system/1/flake.lock
# ln -s ../../../../lib/templateFloxEnv/flake.nix tests/upgrade/$system/1/flake.nix
# ln -s ../../../../lib/templateFloxEnv/flake.lock tests/upgrade/$system/2/flake.lock
# ln -s ../../../../lib/templateFloxEnv/flake.nix tests/upgrade/$system/2/flake.nix
# rm upgrade.tar
# flox unsubscribe nixpkgs-flox-upgrade-test
@test "flox upgrade" {
  case "$NIX_SYSTEM" in
  aarch64-darwin)
    RG_PATH="/nix/store/ix73alhygpflvq50fimdgwl1x2f8yv7y-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/8nv1g4ymxi2f96pbl1jy9h625v2risd8-curl-7.86.0-bin/bin/curl"
    ;;
  x86_64-darwin)
    RG_PATH="/nix/store/1h3ymrn6mai4y3z1gi13yl5alf15xixd-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/sg2j79m7vbvynfd9kpn2hycva2c2y92w-curl-7.86.0-bin/bin/curl"
    ;;
  aarch64-linux)
    RG_PATH="/nix/store/zcq437znz7080wc7gbhijdm5x66qk5lj-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/0b3a9wbhss293wd8qv6q6gfh2wgk34c6-curl-7.86.0-bin/bin/curl"
    ;;
  x86_64-linux)
    RG_PATH="/nix/store/cv1ska2lnafi6l650d4943bm0r3qvixy-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/m2h1p50yvcq5j9b3hkrwqnmrr9pbkzpz-curl-7.86.0-bin/bin/curl"
    ;;
  *)
    echo "unsupported system for upgrade test"
    exit 1
    ;;
  esac

  # TODO move this later in the test. Right now floxEnvs fetch flakes even for catalog entries,
  # which they shouldn't
  run $FLOX_BIN subscribe nixpkgs-flox-upgrade-test github:flox/nixpkgs-flox/master
  assert_success
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C $TESTS_DIR/upgrade/$NIX_SYSTEM . | $FLOX_BIN --bash-passthru import -e _upgrade_testing_"
  assert_success
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which rg)'
  assert_output --partial "$RG_PATH"
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which curl)'
  assert_output --partial "$CURL_PATH"

  # upgrade ripgrep but not curl
  run $FLOX_BIN --bash-passthru upgrade -e _upgrade_testing_ ripgrep
  assert_success
  assert_output --partial "Environment '_upgrade_testing_' upgraded."
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which rg)'
  ! assert_output --partial "$RG_PATH"
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which curl)'
  assert_output --partial "$CURL_PATH"

  # upgrade everything
  run $FLOX_BIN --bash-passthru --debug upgrade -e _upgrade_testing_
  assert_success
  assert_output --partial "Environment '_upgrade_testing_' upgraded."
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which rg)'
  ! assert_output --partial "$RG_PATH"
  run $FLOX_BIN --bash-passthru activate -e _upgrade_testing_ -- sh -xc 'realpath $(which curl)'
  ! assert_output --partial "$CURL_PATH"

  # teardown
  run $FLOX_BIN unsubscribe nixpkgs-flox-upgrade-test
  assert_success
  run $FLOX_BIN --bash-passthru delete -e _upgrade_testing_ -f
  assert_success
}

@test "flox upgrade of nonexistent environment should fail" {
  run $FLOX_BIN --bash-passthru upgrade -e does-not-exist
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_BIN git branch -a | grep -q does-not-exist"
  assert_failure
  # -- output differs --
  # expected (0 lines):
  #
  # actual (2 lines):
  #   Updating "/tmp/tmp.KrigRID1eZ/.config/flox/gitconfig"
  #   Updating /tmp/tmp.KrigRID1eZ/.config/flox/gitconfig
  # --
  #assert_output - < /dev/null
}

@test "flox rollback of nonexistent environment should fail" {
  run $FLOX_BIN --bash-passthru rollback -e does-not-exist
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_BIN git branch -a | grep -q does-not-exist"
  assert_failure
  # -- output differs --
  # expected (0 lines):
  #
  # actual (2 lines):
  #   Updating "/tmp/tmp.KrigRID1eZ/.config/flox/gitconfig"
  #   Updating /tmp/tmp.KrigRID1eZ/.config/flox/gitconfig
  # --
  #assert_output - < /dev/null
}

@test "flox rollback" {
  run $FLOX_BIN --bash-passthru rollback -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 6 to 5."
}

@test "flox list after rollback should reflect generation 2" {
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox rollback --to 4" {
  run $FLOX_BIN --bash-passthru rollback --to 4 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "Rolled back environment '$TEST_ENVIRONMENT' from generation [0-9]+ to 4."
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  4"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox switch-generation 2" {
  run $FLOX_BIN --bash-passthru switch-generation 2 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --regexp "Switched environment '$TEST_ENVIRONMENT' from generation [0-9]+ to 2."
  run $FLOX_BIN --bash-passthru list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  2"
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  refute_output --partial "stable.nixpkgs-flox.cowsay"
  refute_output --partial "stable.nixpkgs-flox.dasel"
  refute_output --partial "stable.nixpkgs-flox.jq"
}

@test "flox switch-generation 9999" {
  run $FLOX_BIN --bash-passthru switch-generation 9999 -e $TEST_ENVIRONMENT
  assert_failure
  assert_output --partial "ERROR: could not find environment data for generation '9999'"
}

@test "flox environments takes no arguments" {
  run $FLOX_BIN --bash-passthru environments -e $TEST_ENVIRONMENT
  assert_failure
  # assert_output --partial '`-e` is not expected in this context' # this is a bpaf error, cant expect that with --bash-passthru`
}

@test "flox environments should at least contain $TEST_ENVIRONMENT" {
  run $FLOX_BIN --bash-passthru --debug environments
  assert_success
  assert_output --partial "/$TEST_ENVIRONMENT"
  assert_output --partial "Alias     $TEST_ENVIRONMENT"
}

@test "flox delete local only" {
  run $FLOX_BIN --bash-passthru delete -e $TEST_ENVIRONMENT -f
  assert_success
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}

@test "flox install by /nix/store path" {
  run $FLOX_BIN --bash-passthru install -e $TEST_ENVIRONMENT "$HELLO_PACKAGE"
  assert_success
  assert_output --partial "Installed '$HELLO_PACKAGE' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox install by nixpkgs flake" {
  run $FLOX_BIN --bash-passthru install -e $TEST_ENVIRONMENT "nixpkgs#cowsay"
  assert_success
  assert_output --partial "Installed 'nixpkgs#cowsay' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox export to $FLOX_TEST_HOME/floxExport.tar" {
  run sh -c "$FLOX_BIN --bash-passthru export -e $TEST_ENVIRONMENT > $FLOX_TEST_HOME/floxExport.tar"
  assert_success
}

@test "flox.nix after installing by nixpkgs flake should contain package" {
  EDITOR='cat' run $FLOX_BIN --bash-passthru edit -e "$TEST_ENVIRONMENT"
  assert_success
  assert_output --partial 'packages.nixpkgs.cowsay = {};'
  refute_output --partial "created generation"
}

@test "flox remove by nixpkgs flake 1" {
  run $FLOX_BIN --bash-passthru remove -e "$TEST_ENVIRONMENT" "nixpkgs#cowsay"
  assert_success
  assert_output --partial "Removed 'nixpkgs#cowsay' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox list after remove by nixpkgs flake 2 should not contain package" {
  run $FLOX_BIN --bash-passthru list -e "$TEST_ENVIRONMENT"
  assert_success
  assert_output --regexp "[0-9]+ +$HELLO_PACKAGE +$HELLO_PACKAGE_FIRST8"
  refute_output --partial "nixpkgs#cowsay"
  refute_output --partial "stable.nixpkgs-flox.cowsay"
}

@test "flox import from $FLOX_TEST_HOME/floxExport.tar" {
  run sh -c "$FLOX_BIN --bash-passthru import -e $TEST_ENVIRONMENT < $FLOX_TEST_HOME/floxExport.tar"
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' imported."
}

@test "tear down install test state" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_BIN --bash-passthru delete -e $TEST_ENVIRONMENT --origin -f"
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim:ts=4:noet:syntax=bash
