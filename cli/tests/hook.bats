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

# Deactivate-action handling in hook-env is not gated, but the auto-activation
# placeholder (_FLOX_HOOK_FIRED) still is.
# TODO: Remove this test when the auto_activate feature flag is removed.
# bats test_tags=hook:hook-env
@test "'flox hook-env' succeeds without auto_activate feature flag but doesn't auto-activate" {
  unset FLOX_FEATURES_AUTO_ACTIVATE
  run "$FLOX_BIN" hook-env --shell bash --shell-pid "$$" --invocation-type inplace
  assert_success
  refute_output --partial "_FLOX_HOOK_FIRED"
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

# ---------------------------------------------------------------------------- #
# Plain `flox deactivate` is serviced by the prompt hook
# ---------------------------------------------------------------------------- #
#
# `flox deactivate` (no `--print-script`) writes a prompt-hook action file; the
# next time the prompt hook runs `flox hook-env`, it restores the environment in
# place. We fire `_flox_hook` manually to stand in for the next prompt.

set_test_var_manifest() {
  cat << "EOF" | "$FLOX_BIN" edit -f -
version = 1

[vars]
TEST_VAR = "modified"
EOF
}

# bats test_tags=hook:deactivate:bash
@test "bash: plain 'flox deactivate' restores env via the prompt hook" {
  project_setup
  set_test_var_manifest

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    export TEST_VAR=original
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    echo \"during:\$TEST_VAR\"
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after:\$TEST_VAR\"
  "
  assert_success
  assert_output --partial "during:modified"
  assert_output --partial "after:original"
}

# bats test_tags=hook:deactivate:zsh
@test "zsh: plain 'flox deactivate' restores env via the prompt hook" {
  project_setup
  set_test_var_manifest

  run --separate-stderr zsh -c "
    export FLOX_SHELL=\$(which zsh)
    export TEST_VAR=original
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    echo \"during:\$TEST_VAR\"
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after:\$TEST_VAR\"
  "
  assert_success
  assert_output --partial "during:modified"
  assert_output --partial "after:original"
}

# bats test_tags=hook:deactivate:fish
@test "fish: plain 'flox deactivate' restores env via the prompt hook" {
  project_setup
  set_test_var_manifest

  run --separate-stderr fish -c "
    set -gx TEST_VAR original
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    echo \"during:\$TEST_VAR\"
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after:\$TEST_VAR\"
  "
  assert_success
  assert_output --partial "during:modified"
  assert_output --partial "after:original"
}

# bats test_tags=hook:deactivate:tcsh
@test "tcsh: plain 'flox deactivate' restores env via the prompt hook" {
  project_setup
  set_test_var_manifest

  run --separate-stderr tcsh -c "
    setenv TEST_VAR original
    eval \"\`$FLOX_BIN activate -d $PROJECT_DIR\`\"
    echo \"during:\$TEST_VAR\"
    $FLOX_BIN deactivate
    precmd
    echo \"after:\$TEST_VAR\"
  "
  assert_success
  assert_output --partial "during:modified"
  assert_output --partial "after:original"
}

# ---------------------------------------------------------------------------- #
# Plain `flox deactivate` errors when no compatible prompt hook will consume it
# ---------------------------------------------------------------------------- #
#
# Writing the action file is only useful if this shell has a compatible prompt
# hook to read it. `flox deactivate` checks the exported _FLOX_PROMPT_HOOK_VERSION
# (and the disable_hook config) and errors rather than claiming success for a
# deactivation that would never happen.

# bats test_tags=hook:deactivate:not-set-up
@test "plain 'flox deactivate' errors when the prompt hook is not set up" {
  project_setup

  # Activate (which exports the version marker), then clear the marker to
  # simulate a shell with no prompt hook registered.
  run bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    unset _FLOX_PROMPT_HOOK_VERSION
    $FLOX_BIN deactivate
  "
  assert_failure
  assert_output --partial "is not set up in this shell"
}

# bats test_tags=hook:deactivate:incompatible
@test "plain 'flox deactivate' errors when the prompt hook version is incompatible" {
  project_setup

  run bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    export _FLOX_PROMPT_HOOK_VERSION=99
    $FLOX_BIN deactivate
  "
  assert_failure
  assert_output --partial "incompatible version of Flox"
}

# bats test_tags=hook:deactivate:disabled
@test "plain 'flox deactivate' errors when the prompt hook is disabled in config" {
  project_setup
  "$FLOX_BIN" config --set disable_hook true

  run bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    $FLOX_BIN deactivate
  "
  assert_failure
  assert_output --partial "prompt hook is disabled"
}
