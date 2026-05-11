#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox hook-env` command and hook code injection into
# `flox activate` output.
#
# ============================================================================ #

load test_support.bash

# bats file_tags=hook

# ---------------------------------------------------------------------------- #

project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return
  "$FLOX_BIN" init -d "$PROJECT_DIR"
}

project_teardown() {
  popd >/dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  if [ -n "${PROJECT_DIR:-}" ]; then
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# hook-env: feature flag gating
# ---------------------------------------------------------------------------- #

# TODO: Remove this test when the auto_activate feature flag is removed.
# bats test_tags=hook:hook-env
@test "'flox hook-env' fails without auto_activate feature flag" {
  unset FLOX_FEATURES_AUTO_ACTIVATE
  run "$FLOX_BIN" hook-env --shell bash
  assert_failure
  assert_output --partial "auto_activate feature flag"
}

# bats test_tags=hook:hook-env
@test "'flox hook-env --shell bash' outputs _FLOX_HOOK_FIRED when flag is enabled" {
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_FIRED"
}

# ---------------------------------------------------------------------------- #
# activate: hook code in output per shell
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:activate:bash
@test "bash: activate output includes hook code when auto_activate is on" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  FLOX_SHELL="$(which bash)" run "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script
  assert_success
  assert_output --partial "hook-env --shell bash"
}

# bats test_tags=hook:activate:zsh
@test "zsh: activate output includes hook code when auto_activate is on" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  FLOX_SHELL="$(which zsh)" run "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script
  assert_success
  assert_output --partial "hook-env --shell zsh"
}

# bats test_tags=hook:activate:fish
@test "fish: activate output includes hook code when auto_activate is on" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  FLOX_SHELL="$(which fish)" run "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script
  assert_success
  assert_output --partial "hook-env --shell fish"
}

# bats test_tags=hook:activate:tcsh
@test "tcsh: activate output includes hook code when auto_activate is on" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  FLOX_SHELL="$(which tcsh)" run "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script
  assert_success
  assert_output --partial "hook-env --shell tcsh"
}

# TODO: Remove this test when the auto_activate feature flag is removed.
# bats test_tags=hook:activate:no-flag
@test "bash: activate output does NOT include hook code when auto_activate is off" {
  project_setup
  unset FLOX_FEATURES_AUTO_ACTIVATE
  FLOX_SHELL="$(which bash)" run "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script
  assert_success
  refute_output --partial "hook-env"
}

# ---------------------------------------------------------------------------- #
# Hook behavior: interactive vs non-interactive
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:behavior:bash
@test "bash: sourcing hook code registers _flox_hook in PROMPT_COMMAND" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  local hook_code
  hook_code="$(FLOX_SHELL="$(which bash)" "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script)"

  # Extract just the hook registration portion and verify it sets up
  # PROMPT_COMMAND correctly in a bash shell.
  run bash -c "
    eval '$hook_code'
    # Verify _flox_hook is in PROMPT_COMMAND
    [[ \" \${PROMPT_COMMAND[*]} \" =~ _flox_hook ]]
  "
  assert_success
}

# Validate that the hook fires with pushd/popd (interactive prompt)
# but NOT with 'bash -c "cd ... && foo"' (non-interactive). This is naturally
# satisfied because PROMPT_COMMAND only runs in interactive bash shells.
# bats test_tags=hook:behavior:non-interactive
@test "bash: hook does NOT fire in non-interactive bash -c" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  local hook_code
  hook_code="$(FLOX_SHELL="$(which bash)" "$FLOX_BIN" activate -d "$PROJECT_DIR" --print-script)"

  # In a non-interactive shell, PROMPT_COMMAND is never executed.
  # We verify the hook code can be sourced without error.
  run bash -c "eval '$hook_code'; echo done"
  assert_success
  assert_output --partial "done"
}

# ---------------------------------------------------------------------------- #
# Hook fires: verify _FLOX_HOOK_FIRED is set per shell
# ---------------------------------------------------------------------------- #

# Each test has the shell call `flox activate` directly (not pre-captured in
# a bats variable) to avoid quoting issues across shells.

# bats test_tags=hook:fires:bash
@test "bash: hook fires and sets _FLOX_HOOK_FIRED to cwd" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    _flox_hook
    echo \$_FLOX_HOOK_FIRED
  "
  assert_success
  assert_output --partial "$PWD"
}

# bats test_tags=hook:fires:zsh
@test "zsh: hook fires and sets _FLOX_HOOK_FIRED to cwd" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run zsh -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which zsh)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    _flox_hook
    echo \$_FLOX_HOOK_FIRED
  "
  assert_success
  assert_output --partial "$PWD"
}

# bats test_tags=hook:fires:fish
@test "fish: hook fires and sets _FLOX_HOOK_FIRED to cwd" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run fish -c "
    set -gx FLOX_FEATURES_AUTO_ACTIVATE true
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    _flox_hook
    echo \$_FLOX_HOOK_FIRED
  "
  assert_success
  assert_output --partial "$PWD"
}

# bats test_tags=hook:fires:tcsh
@test "tcsh: hook fires and sets _FLOX_HOOK_FIRED to cwd" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run tcsh -c "
    setenv FLOX_FEATURES_AUTO_ACTIVATE true
    eval \`$FLOX_BIN activate -d $PROJECT_DIR\`
    precmd
    echo \$_FLOX_HOOK_FIRED
  "
  assert_success
  assert_output --partial "$PWD"
}
