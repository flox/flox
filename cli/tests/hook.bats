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
    printenv _FLOX_HOOK_FIRED
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
    printenv _FLOX_HOOK_FIRED
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
    printenv _FLOX_HOOK_FIRED
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
    eval \"\`$FLOX_BIN activate -d $PROJECT_DIR\`\"
    precmd
    printenv _FLOX_HOOK_FIRED
  "
  assert_success
  assert_output --partial "$PWD"
}

# ---------------------------------------------------------------------------- #
# Hook auto-fires: verify the prompt hook triggers without manual invocation
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-fires
@test "bash: hook auto-fires via PROMPT_COMMAND in interactive shell" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  mkdir -p "$PROJECT_DIR/subdir"

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR" 'echo _FLOX_HOOK_FIRED="$_FLOX_HOOK_FIRED" && pushd subdir >/dev/null && echo _FLOX_HOOK_FIRED="$_FLOX_HOOK_FIRED" && popd >/dev/null && echo _FLOX_HOOK_FIRED="$_FLOX_HOOK_FIRED"'
  # All three echos should show the project dir — the hook fired before the
  # compound command and the value doesn't change mid-pipeline.
  local expected="_FLOX_HOOK_FIRED=$(realpath "$PROJECT_DIR")"
  local count
  count=$(sed 's/^[[:space:]]*//;s/[[:space:]]*$//' <<< "$output" | grep -cFx "$expected")
  assert_equal "$count" "3"
}
