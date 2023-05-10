#!/usr/bin/env bats
#
# flox CLI tests run in two contexts:
# - unit tests only to be run from within package build
# - unit and integration tests to be run from command line
#
bats_load_library bats-support
bats_load_library bats-assert
bats_require_minimum_version 1.5.0

load test_support.bash

@test "flox package sanity check" {
  # directories
  [ -d $FLOX_PACKAGE/bin ]
  [ -d $FLOX_PACKAGE/libexec ]
  [ -d $FLOX_PACKAGE/libexec/flox ]
  [ -d $FLOX_PACKAGE/etc ]
  [ -d $FLOX_PACKAGE/etc/flox.zdotdir ]
  [ -d $FLOX_PACKAGE/lib ]
  [ -d $FLOX_PACKAGE/share ]
  [ -d $FLOX_PACKAGE/share/man ]
  [ -d $FLOX_PACKAGE/share/man/man1 ]
  [ -d $FLOX_PACKAGE/share/bash-completion ]
  [ -d $FLOX_PACKAGE/share/bash-completion/completions ]
  # executables
  [ -x $FLOX_CLI ]
  [ -x $FLOX_PACKAGE/libexec/flox/gh ]
  [ -x $FLOX_PACKAGE/libexec/flox/nix ]
  [ -x $FLOX_PACKAGE/libexec/flox/flox ]
  # Could go on ...
}

@test "assert testing home $FLOX_TEST_HOME" {
  run sh -c "test -d $FLOX_TEST_HOME"
  assert_success
}

@test "flox --prefix" {
  run $FLOX_CLI --prefix
  assert_success
  assert_output $FLOX_PACKAGE
}

@test "flox generate config files in $FLOX_CONFIG_HOME" {
  # The rust wrapper will not forward all commands to flox (bash)
  # Help messages for instance are generated entirely by the argument parsing step,
  # that precedes any command processing.
  # As such this tests fails to see the "Updating ..." messages if used with `--help`.
  # The first test forwarding to flox (subscribe, below) will and fails as well.
  #
  # This test will work until channels will be implemented in rust.
  # At which point the messaging may change as well.
  run $FLOX_CLI channels
  assert_success
  assert_output --partial "Updating $FLOX_CONFIG_HOME/nix.conf"
  assert_output --partial "Updating $FLOX_CONFIG_HOME/gitconfig"
}

@test "flox git remote -v" {
  run $FLOX_CLI git remote -v
  assert_success
  assert_output - < /dev/null
}

@test "flox --help" {
  run $FLOX_CLI --help
  assert_success
  # the rust implementation generates its USAGE/help internally
  if [ "$FLOX_IMPLEMENTATION" != "rust" ]; then
    assert_output - <tests/usage.out
  fi
}

@test "flox eval" {
  # Evaluate a Nix expression given on the command line:
  run $FLOX_CLI eval --expr '1 + 2'
  assert_success
  echo 3 | assert_output -

  # Evaluate a Nix expression to JSON:
  run $FLOX_CLI eval --json --expr '{ x = 1; }'
  assert_success
  echo '{"x":1}' | assert_output -

  # Evaluate a Nix expression from a file:
  run $FLOX_CLI eval -f ./tests tests.name
  assert_success
  echo '"tests-1.2.3"' | assert_output -

  # Get the current version of the nixpkgs flake:
  run $FLOX_CLI eval --raw 'nixpkgs#lib.version'
  assert_success
  # something like "23.05pre-git"
  assert_output --regexp "[0-9][0-9].[0-9][0-9]"

  # Print the store path of the Hello package:
  run $FLOX_CLI eval --raw nixpkgs#hello
  assert_success
  assert_output --regexp "/nix/store/.*-hello-"

  # Get a list of checks in the nix flake:
  run $FLOX_CLI eval github:nixos/nix#checks.x86_64-linux --apply builtins.attrNames
  assert_success
  # Unfortunately we need to do a partial match because our attempt
  # to override the nixpkgs input throws a warning on a non-capacitated
  # flake.
  assert_output --partial '[ "binaryTarball" "dockerImage" "installTests" "nixpkgsLibTests" "perlBindings" ]'

  # Generate a directory with the specified contents:
  run $FLOX_CLI eval --write-to ./tests/out --expr '{ foo = "bar"; subdir.bla = "123"; }'
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
  run $FLOX_CLI subscribe flox-examples github:flox-examples/floxpkgs
  assert_success
  assert_output --partial "subscribed channel 'flox-examples'"
}

@test "flox unsubscribe public" {
  run $FLOX_CLI unsubscribe flox-examples
  assert_success
  assert_output --partial "unsubscribed from channel 'flox-examples'"
}

@test "assert not logged into github" {
  run $FLOX_CLI gh auth status
  assert_failure
  assert_output --partial "You are not logged into any GitHub hosts. Run gh auth login to authenticate."
}

@test "assert no access to private repository" {
  # otherwise a cached version of the private repo may be used
  run unlink $XDG_CACHE_HOME/nix
  assert_success
  run $FLOX_CLI flake metadata github:flox-examples/floxpkgs-private --no-eval-cache --no-write-lock-file --json
  assert_failure
  run ln -s ~/.cache/nix $XDG_CACHE_HOME/nix
  assert_success
}

@test "flox subscribe private without creds" {
  run $FLOX_CLI subscribe flox-examples-private github:flox-examples/floxpkgs-private
  assert_failure
  assert_output --partial 'ERROR: could not verify channel URL: "github:flox-examples/floxpkgs-private"'
}

# These next two tests are annoying:
# - the `gh` tool requires GH_CONFIG_DIR
# - while `nix` requires XDG_CONFIG_HOME
#   - ... and because `nix` invokes `gh`, just provide them both
@test "assert can log into github GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI gh auth status"
  assert_success
  assert_output --partial "✓ Logged in to github.com as"
}

@test "flox subscribe private with creds GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI subscribe flox-examples-private github:flox-examples/floxpkgs-private"
  assert_success
  assert_output --partial "subscribed channel 'flox-examples-private'"
}

# Keep environment in next test to prevent nix.conf rewrite warning.
@test "flox unsubscribe private" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI unsubscribe flox-examples-private"
  assert_success
  assert_output --partial "unsubscribed from channel 'flox-examples-private'"
}

@test "flox create -e $TEST_ENVIRONMENT" {
  run $FLOX_CLI create -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "created environment $TEST_ENVIRONMENT ($NIX_SYSTEM)"
}

@test "flox create -e $TEST_ENVIRONMENT fails when run again" {
  run $FLOX_CLI create -e $TEST_ENVIRONMENT
  assert_failure
  assert_output --partial "ERROR: environment $TEST_ENVIRONMENT ($NIX_SYSTEM) already exists"
}

@test "flox install hello" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT hello
  assert_success
  assert_output --partial "Installed 'hello' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox export -> import is a no-op" {
  run sh -c "$FLOX_CLI export -e $TEST_ENVIRONMENT | $FLOX_CLI --debug import -e $TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "No environment changes detected"
}

@test "flox install nixpkgs-flox.hello" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT nixpkgs-flox.hello
  assert_success
  assert_output --partial "No change! Package(s) 'nixpkgs-flox.hello' already installed into '$TEST_ENVIRONMENT' environment."
}

@test "flox install stable.nixpkgs-flox.hello" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT stable.nixpkgs-flox.hello
  assert_success
  assert_output --partial "No change! Package(s) 'stable.nixpkgs-flox.hello' already installed into '$TEST_ENVIRONMENT' environment."
}

# A rose by any other name ...
@test "flox subscribe nixpkgs-flox-dup" {
  run $FLOX_CLI subscribe nixpkgs-flox-dup github:flox/nixpkgs-flox/master
  assert_success
  assert_output --partial "subscribed channel 'nixpkgs-flox-dup'"
}

@test "flox install stable.nixpkgs-flox-dup.hello" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT stable.nixpkgs-flox-dup.hello
  assert_failure
  assert_output --regexp ".*error: package nixpkgs-flox.$NIX_SYSTEM.stable.hello.latest is identical to package nixpkgs-flox-dup.$NIX_SYSTEM.stable.hello.latest"
}

@test "flox list after install should contain hello" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  2"
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
}

@test "flox install cowsay jq dasel" {
  run $FLOX_CLI --debug install -e $TEST_ENVIRONMENT cowsay jq dasel
  assert_success
  assert_output --partial "created generation 3"
}

@test "flox list after install should contain cowsay and hello" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  3"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox activate can invoke hello and cowsay" {
  run $FLOX_CLI activate -e $TEST_ENVIRONMENT -- sh -c 'hello | cowsay'
  assert_success
  assert_output - < tests/hello-cowsay.out
}

@test "flox edit remove hello" {
  EDITOR=./tests/remove-hello run $FLOX_CLI edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified."
}

@test "verify flox edit removed hello from manifest.json" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  4"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "verify flox edit removed hello from flox.nix" {
  EDITOR=cat run $FLOX_CLI edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial 'nixpkgs-flox.cowsay = {'
  assert_output --partial 'nixpkgs-flox.dasel = {'
  ! assert_output --partial 'nixpkgs-flox.hello = {'
  assert_output --partial 'nixpkgs-flox.jq = {'
  ! assert_output --partial "created generation"
}

@test "flox edit add hello" {
  EDITOR=./tests/add-hello run $FLOX_CLI edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' modified."
}

@test "verify flox edit added hello to manifest.json" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  5"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "verify flox edit added hello to flox.nix" {
  EDITOR=cat run $FLOX_CLI edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial 'nixpkgs-flox.cowsay = {'
  assert_output --partial 'nixpkgs-flox.dasel = {'
  assert_output --partial 'nixpkgs-flox.hello = {'
  assert_output --partial 'nixpkgs-flox.jq = {'
  ! assert_output --partial "created generation"
}

@test "flox edit preserves comments" {
  EDIT_ENVIRONMENT=_edit_testing_
  run $FLOX_CLI create -e "$EDIT_ENVIRONMENT"
  assert_success

  EDITOR=./tests/add-comment run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial "Environment '$EDIT_ENVIRONMENT' modified."

  EDITOR=cat run $FLOX_CLI edit -e "$EDIT_ENVIRONMENT"
  assert_success
  assert_output --partial "# test comment"

  run $FLOX_CLI destroy --force -e "$EDIT_ENVIRONMENT"
  assert_success
}

@test "flox remove hello" {
  run $FLOX_CLI remove -e $TEST_ENVIRONMENT hello
  assert_success
  assert_output --partial "Removed 'hello' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox list after remove should not contain hello" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  6"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox list of generation 3 should contain hello" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT 3
  assert_success
  assert_output --partial "Curr Gen  3"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox history should contain the install and removal of stable.nixpkgs-flox.hello" {
  run $FLOX_CLI history -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "removed stable.nixpkgs-flox.hello"
  assert_output --partial "installed stable.nixpkgs-flox.cowsay stable.nixpkgs-flox.jq stable.nixpkgs-flox.dasel"
  assert_output --partial "installed stable.nixpkgs-flox.hello"
  assert_output --partial "created environment"
}

@test "flox remove from nonexistent environment should fail" {
  run $FLOX_CLI remove -e does-not-exist hello
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_CLI git branch -a | grep -q does-not-exist"
  assert_failure
  assert_output - < /dev/null
}

@test "flox remove channel package by index" {
  TEST_CASE_ENVIRONMENT=$(uuid)

  run $FLOX_CLI install -e $TEST_CASE_ENVIRONMENT hello
  assert_success

  run $FLOX_CLI list -e $TEST_CASE_ENVIRONMENT
  assert_success
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX

  run $FLOX_CLI remove -e $TEST_CASE_ENVIRONMENT 0
  assert_success
  assert_output --partial "Removed '0' package(s) from '$TEST_CASE_ENVIRONMENT' environment."

  run $FLOX_CLI list -e $TEST_CASE_ENVIRONMENT
  assert_success
  ! assert_output --partial "stable.nixpkgs-flox.hello"

  # teardown
  run $FLOX_CLI destroy -e $TEST_CASE_ENVIRONMENT -f
  assert_success
}

@test "flox remove flake package by index" {
  TEST_CASE_ENVIRONMENT=$(uuid)

  run $FLOX_CLI install -e $TEST_CASE_ENVIRONMENT nixpkgs#hello
  assert_success

  run $FLOX_CLI list -e $TEST_CASE_ENVIRONMENT
  assert_success
  assert_output --regexp "0  nixpkgs#hello  hello-"$VERSION_REGEX

  run $FLOX_CLI remove -e $TEST_CASE_ENVIRONMENT 0
  assert_success
  assert_output --partial "Removed '0' package(s) from '$TEST_CASE_ENVIRONMENT' environment."

  run $FLOX_CLI list -e $TEST_CASE_ENVIRONMENT
  assert_success
  ! assert_output --partial "nixpkgs#hello"

  # teardown
  run $FLOX_CLI destroy -e $TEST_CASE_ENVIRONMENT -f
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
  case $NIX_SYSTEM in
  aarch64-darwin)
    RG_PATH="/nix/store/ix73alhygpflvq50fimdgwl1x2f8yv7y-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/8nv1g4ymxi2f96pbl1jy9h625v2risd8-curl-7.86.0-bin/bin/curl"
    CURL_PATH_2="/nix/store/8nv1g4ymxi2f96pbl1jy9h625v2risd8-curl-7.86.0-bin/bin/curl"
    ;;
  aarch64-linux)
    RG_PATH="/nix/store/zcq437znz7080wc7gbhijdm5x66qk5lj-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/0b3a9wbhss293wd8qv6q6gfh2wgk34c6-curl-7.86.0-bin/bin/curl"
    CURL_PATH_2="/nix/store/0b3a9wbhss293wd8qv6q6gfh2wgk34c6-curl-7.86.0-bin/bin/curl"
    ;;
  x86_64-linux)
    RG_PATH="/nix/store/cv1ska2lnafi6l650d4943bm0r3qvixy-ripgrep-13.0.0/bin/rg"
    CURL_PATH="/nix/store/b7xwyhb5zy4x26jvk9vl84ihb7gcijrn-curl-7.86.0/bin/curl"
    CURL_PATH_2="/nix/store/m2h1p50yvcq5j9b3hkrwqnmrr9pbkzpz-curl-7.86.0-bin/bin/curl"
    ;;
  *)
    echo "unsupported system for upgrade test"
    exit 1
    ;;
  esac

  # TODO move this later in the test. Right now floxEnvs fetch flakes even for catalog entries,
  # which they shouldn't
  run $FLOX_CLI subscribe nixpkgs-flox-upgrade-test github:flox/nixpkgs-flox/master
  assert_success
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C ./tests/upgrade/$NIX_SYSTEM . | $FLOX_CLI import -e _upgrade_testing_"
  assert_success
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which rg)'
  assert_output "$RG_PATH"
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which curl)'
  assert_output "$CURL_PATH"

  # upgrade ripgrep but not curl
  run $FLOX_CLI upgrade -e _upgrade_testing_ ripgrep
  assert_success
  assert_output --partial "Environment '_upgrade_testing_' upgraded."
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which rg)'
  ! assert_output --partial "$RG_PATH"
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which curl)'
  # Even though it hasn't been upgraded, the path to curl can still change due
  # to how it's wrapped by buildenv
  assert_output --partial "$CURL_PATH_2"

  # upgrade everything
  run $FLOX_CLI upgrade -e _upgrade_testing_
  assert_success
  assert_output --partial "Environment '_upgrade_testing_' upgraded."
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which rg)'
  ! assert_output --partial "$RG_PATH"
  run $FLOX_CLI activate -e _upgrade_testing_ -- sh -c 'readlink $(which curl)'
  ! assert_output --partial "$CURL_PATH"
  ! assert_output --partial "$CURL_PATH_2"

  # teardown
  run $FLOX_CLI unsubscribe nixpkgs-flox-upgrade-test
  assert_success
  run $FLOX_CLI destroy -e _upgrade_testing_ -f
  assert_success
}

@test "flox upgrade of nonexistent environment should fail" {
  run $FLOX_CLI upgrade -e does-not-exist
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_CLI git branch -a | grep -q does-not-exist"
  assert_failure
  assert_output - < /dev/null
}

@test "flox rollback of nonexistent environment should fail" {
  run $FLOX_CLI rollback -e does-not-exist
  assert_failure
  assert_output --partial "ERROR: environment does-not-exist ($NIX_SYSTEM) does not exist"
  run sh -c "$FLOX_CLI git branch -a | grep -q does-not-exist"
  assert_failure
  assert_output - < /dev/null
}

@test "flox rollback" {
  run $FLOX_CLI rollback -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 6 to 5."
}

@test "flox list after rollback should reflect generation 2" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  5"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --regexp "3  stable.nixpkgs-flox.jq +"$VERSION_REGEX
}

@test "flox rollback --to 4" {
  run $FLOX_CLI rollback --to 4 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 5 to 4."
}

@test "flox list after rollback --to 4 should reflect generation 4" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  4"
  assert_output --regexp "0  stable.nixpkgs-flox.cowsay +"$VERSION_REGEX
  assert_output --regexp "1  stable.nixpkgs-flox.dasel +"$VERSION_REGEX
  assert_output --regexp "2  stable.nixpkgs-flox.jq +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox switch-generation 2" {
  run $FLOX_CLI switch-generation 2 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Switched environment '$TEST_ENVIRONMENT' from generation 4 to 2."
}

@test "flox list after switch-generation 2 should reflect generation 2" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  2"
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.cowsay"
  ! assert_output --partial "stable.nixpkgs-flox.dasel"
  ! assert_output --partial "stable.nixpkgs-flox.jq"
}

@test "flox rollback to 1" {
  run $FLOX_CLI rollback -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 2 to 1."
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  # generation 1 has no packages
  assert_output --regexp ".*Packages"
}

@test "flox rollback to 0" {
  run $FLOX_CLI rollback -e $TEST_ENVIRONMENT
  assert_failure
  assert_output --partial "ERROR: invalid generation '0'"
}

@test "flox switch-generation 7" {
  run $FLOX_CLI switch-generation 7 -e $TEST_ENVIRONMENT
  assert_failure
  assert_output --partial "ERROR: could not find environment data for generation '7'"
}

@test "flox rollback --to 2" {
  run $FLOX_CLI switch-generation 2 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Switched environment '$TEST_ENVIRONMENT' from generation 1 to 2."
  run $FLOX_CLI rollback --to 2 -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "start and target generations are the same"
}

@test "flox generations" {
  run $FLOX_CLI generations -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Generation 2:"
  assert_output --partial "Path:"
  assert_output --partial "Created:"
  assert_output --partial "Last active:"
  assert_output --partial "Log entries:"
  assert_output --partial "installed stable.nixpkgs-flox.hello"
  assert_output --partial "Generation 3:"
  assert_output --partial "installed stable.nixpkgs-flox.cowsay stable.nixpkgs-flox.jq stable.nixpkgs-flox.dasel"
  assert_output --partial "Generation 4:"
  assert_output --partial "edited declarative profile (generation 4)"
  assert_output --partial "Generation 5:"
  assert_output --partial "edited declarative profile (generation 5)"
  assert_output --partial "Generation 6:"
  assert_output --partial "removed stable.nixpkgs-flox.hello"
}

@test "flox environments takes no arguments" {
  run $FLOX_CLI environments -e $TEST_ENVIRONMENT
  assert_failure
  if [ "$FLOX_IMPLEMENTATION" != "rust" ]; then
    assert_output --partial "ERROR: the 'flox environments' command takes no arguments"
  else
    assert_output "-e is not expected in this context"
  fi
}

@test "flox environments should at least contain $TEST_ENVIRONMENT" {
  run $FLOX_CLI --debug environments
  assert_success
  assert_output --partial "/$TEST_ENVIRONMENT"
  assert_output --partial "Alias     $TEST_ENVIRONMENT"
}

# Again we need github connectivity for this.
@test "flox push" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI --debug push -e $TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "To "
  assert_output --regexp "\* \[new branch\] +origin/.*.$TEST_ENVIRONMENT -> .*.$TEST_ENVIRONMENT"
}

@test "flox destroy local only" {
  run $FLOX_CLI destroy -e $TEST_ENVIRONMENT -f
  assert_success
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}

# ... and this.
@test "flox pull" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI pull -e $TEST_ENVIRONMENT"
  assert_success
  assert_output --partial "To "
  assert_output --regexp "\* \[new branch\] +.*\.$TEST_ENVIRONMENT -> .*\.$TEST_ENVIRONMENT"
}

@test "flox list after flox pull should be exactly as before" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  2"
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  ! assert_output --partial "stable.nixpkgs-flox.cowsay"
  ! assert_output --partial "stable.nixpkgs-flox.dasel"
  ! assert_output --partial "stable.nixpkgs-flox.jq"
}

@test "flox search should return results quickly" {
  # "timeout 15 flox search" does not work? Haven't investigated why, just
  # fall back to doing the math manually and report when it takes too long.
  local -i start
  start=$(date +%s)
  run $FLOX_CLI search hello
  local -i end
  end=$(date +%s)
  assert_success
  assert_output --partial "hello"
  ! assert_output --partial "stable"
  ! assert_output --partial "nixpkgs-flox"
  # Assert we spent less than 15 seconds in the process.
  local -i elapsed
  elapsed=$(($end - $start))
  echo spent $elapsed seconds
  [ $elapsed -lt 15 ]
}

@test "flox install by /nix/store path" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT $FLOX_PACKAGE
  assert_success
  assert_output --partial "Installed '$FLOX_PACKAGE' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox list after installing by store path should contain package" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  7"
  assert_output --regexp "0  stable.nixpkgs-flox.hello +"$VERSION_REGEX
  assert_output --partial "1  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
}

@test "flox remove hello again" {
  run $FLOX_CLI remove -e $TEST_ENVIRONMENT hello
  assert_success
  assert_output --partial "Removed 'hello' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox install by nixpkgs flake" {
  run $FLOX_CLI install -e $TEST_ENVIRONMENT "nixpkgs#hello"
  assert_success
  assert_output --partial "Installed 'nixpkgs#hello' package(s) into '$TEST_ENVIRONMENT' environment."
}

@test "flox list after installing by nixpkgs flake should contain package" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  9"
  assert_output --regexp "0  nixpkgs#hello +hello-"$VERSION_REGEX
  assert_output --partial "1  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox export to $FLOX_TEST_HOME/floxExport.tar" {
  run sh -c "$FLOX_CLI export -e $TEST_ENVIRONMENT > $FLOX_TEST_HOME/floxExport.tar"
  assert_success
}

@test "flox.nix after installing by nixpkgs flake should contain package" {
  EDITOR=cat run $FLOX_CLI edit -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial 'packages.nixpkgs.hello = {};'
  ! assert_output --partial "created generation"
}

@test "flox remove by nixpkgs flake 1" {
  run $FLOX_CLI remove -e $TEST_ENVIRONMENT "nixpkgs#hello"
  assert_success
  assert_output --partial "Removed 'nixpkgs#hello' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox list after remove by nixpkgs flake 1 should not contain package" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  10"
  assert_output --partial "0  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
  ! assert_output --partial "nixpkgs#hello"
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox rollback after flake removal 1" {
  run $FLOX_CLI rollback -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Rolled back environment '$TEST_ENVIRONMENT' from generation 10 to 9."
}

# @test "flox remove by nixpkgs flake 2" {
#   run $FLOX_CLI remove -e $TEST_ENVIRONMENT "legacyPackages.$NIX_SYSTEM.hello"
#   assert_success
#   assert_output --partial "created generation 10"
# }
#
# @test "flox list after remove by nixpkgs flake 2 should not contain package" {
#   run $FLOX_CLI list -e $TEST_ENVIRONMENT
#   assert_success
#   assert_output --partial "Curr Gen  11"
#   assert_output --partial "0  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
#   ! assert_output --partial "flake:nixpkgs#legacyPackages.$NIX_SYSTEM.hello"
#   ! assert_output --partial "stable.nixpkgs-flox.hello"
# }

@test "flox remove by nixpkgs flake 2" {
  run $FLOX_CLI remove -e $TEST_ENVIRONMENT "flake:nixpkgs#legacyPackages.$NIX_SYSTEM.hello"
  assert_success
  assert_output --partial "Removed 'flake:nixpkgs#legacyPackages.$NIX_SYSTEM.hello' package(s) from '$TEST_ENVIRONMENT' environment."
}

@test "flox list after remove by nixpkgs flake 2 should not contain package" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  11"
  assert_output --partial "0  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
  ! assert_output --partial "nixpkgs#hello"
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

# @test "flox switch-generation after flake removal 2" {
#   run $FLOX_CLI rollback -e $TEST_ENVIRONMENT --to 8
#   assert_success
#   assert_output --partial "switched to generation 8"
# }

@test "flox import from $FLOX_TEST_HOME/floxExport.tar" {
  run sh -c "$FLOX_CLI import -e $TEST_ENVIRONMENT < $FLOX_TEST_HOME/floxExport.tar"
  assert_success
  assert_output --partial "Environment '$TEST_ENVIRONMENT' imported."
}

@test "flox list to verify contents of generation 9 at generation 12" {
  run $FLOX_CLI list -e $TEST_ENVIRONMENT
  assert_success
  assert_output --partial "Curr Gen  12"
  assert_output --regexp "0  nixpkgs#hello +hello-"$VERSION_REGEX
  assert_output --partial "1  $FLOX_PACKAGE  $FLOX_PACKAGE_FIRST8"
  ! assert_output --partial "stable.nixpkgs-flox.hello"
}

@test "flox develop setup" {
  # since develop tests use expect, flox thinks it's being used interactively and asks about metrics
  $FLOX_CLI config --setNumber floxMetricsConsent 0
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C $TESTS_DIR/develop ./develop | tar -C $FLOX_TEST_HOME -xf -"
  assert_success
  # note the develop flake may have an out of date lock
}

function assertAndRemoveFiles {
  pushd "$FLOX_TEST_HOME/develop"
    assert [ -h .flox/envs/$NIX_SYSTEM.my-pkg ]
    rm -r .flox
    assert [ -f $FLOX_TEST_HOME/develop/pkgs/my-pkg/catalog.json ]
    rm pkgs/my-pkg/catalog.json
    assert [ -f pkgs/my-pkg/manifest.json ]
    rm pkgs/my-pkg/manifest.json
  popd
}

@test "flox develop no installable" {
  pushd "$FLOX_TEST_HOME/develop"
    run expect "$TESTS_DIR/develop/develop.exp" ""
    assert_success
    assertAndRemoveFiles
  popd
}

@test "flox develop from flake root" {
  pushd "$FLOX_TEST_HOME/develop"
    for attr in "" my-pkg .#my-pkg .#packages.$NIX_SYSTEM.my-pkg "$FLOX_TEST_HOME/develop#my-pkg"; do
      run expect "$TESTS_DIR/develop/develop.exp" "$attr"
      assert_success
      assertAndRemoveFiles
    done
  popd
}

@test "flox develop from flake subdirectory" {
  pushd "$FLOX_TEST_HOME/develop/pkgs"
    for attr in .#my-pkg "$FLOX_TEST_HOME/develop#my-pkg"; do
      run expect "$TESTS_DIR/develop/develop.exp" "$attr"
      assert_success
      assertAndRemoveFiles
    done
  popd
}

@test "flox develop from different directory" {
  pushd "$FLOX_TEST_HOME"
    run expect "$TESTS_DIR/develop/develop.exp" ./develop#my-pkg
    assert_success
  popd
}

@test "flox develop after git init" {
  pushd "$FLOX_TEST_HOME/develop"
    git init
    git add .
    for attr in .#my-pkg "$FLOX_TEST_HOME/develop#my-pkg"; do
      run expect "$TESTS_DIR/develop/develop.exp" "$attr"
      assert_success
      assertAndRemoveFiles
    done
  popd
}

@test "flox develop fails with remote flake" {
  run expect "$TESTS_DIR/develop/develop-fail.exp" "git+ssh://git@github.com/flox/flox-bash-private?dir=tests/develop#my-pkg"
  assert_success
}

@test "flox develop toplevel with package" {
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C $TESTS_DIR/develop ./toplevel-flox-nix-with-pkg | tar -C $FLOX_TEST_HOME -xf -"
  assert_success
  pushd "$FLOX_TEST_HOME/toplevel-flox-nix-with-pkg"
    run expect "$TESTS_DIR/develop/develop.exp" ""
    assert_success
    assert [ -h .flox/envs/$NIX_SYSTEM.default ]
    assert [ -f catalog.json ]
    assert [ -f manifest.json ]
  popd
}

@test "flox develop toplevel" {
  # Note the use of --dereference to copy flake.{nix,lock} as files.
  run sh -c "tar -cf - --dereference --mode u+w -C $TESTS_DIR/develop ./toplevel-flox-nix | tar -C $FLOX_TEST_HOME -xf -"
  assert_success
  pushd "$FLOX_TEST_HOME/toplevel-flox-nix"
    run $FLOX_CLI install -e .#default hello
    assert_success
    # for some reason expect hangs forever when SHELL=zsh and I don't feel like
    # debugging why
    SHELL=bash run expect "$TESTS_DIR/develop/toplevel-flox-nix.exp" ""
    assert_success
    assert [ -h .flox/envs/$NIX_SYSTEM.default ]
    assert [ -f catalog.json ]
    assert [ -f manifest.json ]
  popd
}

@test "flox develop devShell" {
  # Note the use of --dereference to copy flake.lock as file.
  run sh -c "tar -cf - --dereference --mode u+w -C $TESTS_DIR/develop ./devShell | tar -C $FLOX_TEST_HOME -xf -"
  assert_success
  pushd "$FLOX_TEST_HOME/devShell"
    run expect "$TESTS_DIR/develop/devShell.exp" ""
    assert_success
    assert [ ! -h .flox/envs/$NIX_SYSTEM.default ]
    assert [ ! -f catalog.json ]
    assert [ ! -f manifest.json ]
  popd
}

@test "tear down install test state" {
  run sh -c "XDG_CONFIG_HOME=$REAL_XDG_CONFIG_HOME GH_CONFIG_DIR=$REAL_GH_CONFIG_DIR $FLOX_CLI destroy -e $TEST_ENVIRONMENT --origin -f"
  assert_output --partial "WARNING: you are about to delete the following"
  assert_output --partial "Deleted branch"
  assert_output --partial "removed"
}

@test "rm -rf $FLOX_TEST_HOME" {
  run rm -rf $FLOX_TEST_HOME
  assert_success
}

# vim:ts=4:noet:syntax=bash
