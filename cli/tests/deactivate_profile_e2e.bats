#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# End-to-end tests for `[profile.deactivate]`: the full path from manifest
# entry to buildenv emission to runtime hook execution.
#
# These cover the seam between PR-B (schema + buildenv, this PR) and PR-A
# (CLI subcommand + gen_rc plumbing, #4296). Both halves must be present
# for the flow to work end-to-end; this PR is held until after PR-A lands
# and is rebased onto the resulting main, at which point CI exercises
# both sides together.
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=deactivate-profile-e2e

# ---------------------------------------------------------------------------- #

setup_file() {
  common_file_setup
}

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset PROJECT_NAME
}

setup() {
  common_test_setup
  home_setup test
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  cat_teardown_fifo
  if [ -n "${PROJECT_DIR:-}" ]; then
    wait_for_activations "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# Write a manifest exercising [profile.$shell] (sets FOO) and
# [profile.deactivate.$shell] (unsets FOO).
#
# Args:
#   $1  shell name (one of: bash, zsh, fish, tcsh)
#   $2  activate snippet (per-shell syntax to assign FOO)
#   $3  deactivate snippet (per-shell syntax to unset FOO)
_write_profile_deactivate_manifest() {
  local shell="$1"
  local activate_snippet="$2"
  local deactivate_snippet="$3"
  cat <<EOF | "$FLOX_BIN" edit -f -
schema-version = "1.13.0"

[options]

[profile]
$shell = "$activate_snippet"

[profile.deactivate]
$shell = "$deactivate_snippet"
EOF
}

# bats test_tags=deactivate-profile-e2e,deactivate-profile-e2e:bash
@test "bash: profile.deactivate unsets a shell variable on deactivate" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  _write_profile_deactivate_manifest bash "FOO=bar" "unset FOO"
  FLOX_SHELL="bash" run --separate-stderr bash -c '
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:${FOO-unset}"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "after:${FOO-unset}"
  '
  assert_success
  assert_line "during:bar"
  assert_line "after:unset"
}

# bats test_tags=deactivate-profile-e2e,deactivate-profile-e2e:zsh
@test "zsh: profile.deactivate unsets a shell variable on deactivate" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  _write_profile_deactivate_manifest zsh "FOO=bar" "unset FOO"
  FLOX_SHELL="zsh" run --separate-stderr zsh -c '
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:${FOO-unset}"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "after:${FOO-unset}"
  '
  assert_success
  assert_line "during:bar"
  assert_line "after:unset"
}

# bats test_tags=deactivate-profile-e2e,deactivate-profile-e2e:fish
@test "fish: profile.deactivate unsets a shell variable on deactivate" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  _write_profile_deactivate_manifest fish "set FOO bar" "set -e FOO"
  FLOX_SHELL="fish" run --separate-stderr fish -c '
    eval "$($FLOX_BIN activate --print-script)"
    if set -q FOO
      echo "during:$FOO"
    else
      echo "during:unset"
    end
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    if set -q FOO
      echo "after:$FOO"
    else
      echo "after:unset"
    end
  '
  assert_success
  assert_line "during:bar"
  assert_line "after:unset"
}

# bats test_tags=deactivate-profile-e2e,deactivate-profile-e2e:tcsh
@test "tcsh: profile.deactivate unsets a shell variable on deactivate" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  _write_profile_deactivate_manifest tcsh "set FOO=bar" "unset FOO"
  FLOX_SHELL="tcsh" run --separate-stderr tcsh -c '
    eval "`$FLOX_BIN activate --print-script`"
    if ( $?FOO ) then
      echo "during:$FOO"
    else
      echo during:unset
    endif
    eval "`$FLOX_BIN deactivate --print-script $_FLOX_INVOCATION_TYPE`"
    if ( $?FOO ) then
      echo "after:$FOO"
    else
      echo after:unset
    endif
  '
  assert_success
  assert_line "during:bar"
  assert_line "after:unset"
}
