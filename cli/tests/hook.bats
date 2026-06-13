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
# hook-env / deactivate: advisory preamble output is suppressed
# ---------------------------------------------------------------------------- #

# Same shape as the floxhub_setup() token but with an `exp` claim in the past
# (2001-09-09). The CLI decodes tokens without verifying the signature, so the
# signature is arbitrary.
#   { "https://flox.dev/handle": "test", "exp": 1000000000 }
EXPIRED_FLOXHUB_TOKEN="eyJ0eXAiOiJKV1QiLCJhbGciOiJIUzI1NiJ9.eyJodHRwczovL2Zsb3guZGV2L2hhbmRsZSI6InRlc3QiLCJleHAiOjEwMDAwMDAwMDB9.6-nbzFzQEjEX7dfWZFLE-I_qW2N_-9W2HFzzfsquI74"

# The prompt hook runs `flox hook-env` on every prompt inside a command
# substitution that captures only stdout, so anything the CLI preamble prints
# to stderr appears above every prompt — and anything it printed to stdout
# would be eval'd by the shell.
# bats test_tags=hook:hook-env
@test "'flox hook-env' suppresses advisory preamble output" {
  # With the flag set, hook-env legitimately emits an export to stdout.
  unset FLOX_FEATURES_AUTO_ACTIVATE
  _FLOX_FLOXHUB_GIT_URL="https://git.example.invalid/" \
    FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$" --invocation-type inplace
  assert_success
  assert_equal "$stderr" ""
  assert_output ""
}

# An invalid token is normally surfaced and removed from the user's config;
# both must be deferred to the next user-invoked command rather than printing
# and rewriting config on every prompt.
# bats test_tags=hook:hook-env
@test "'flox hook-env' defers invalid token cleanup to user-invoked commands" {
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'floxhub_token = "not-a-jwt"' >> "$FLOX_CONFIG_DIR/flox.toml"

  run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$" --invocation-type inplace
  assert_success
  assert_equal "$stderr" ""
  run grep floxhub_token "$FLOX_CONFIG_DIR/flox.toml"
  assert_success

  run --separate-stderr "$FLOX_BIN" config
  assert_success
  assert_regex "$stderr" "Your FloxHub token is invalid"
  run grep floxhub_token "$FLOX_CONFIG_DIR/flox.toml"
  assert_failure
}

# 'flox deactivate' hands the deactivation off to the prompt hook, so its
# preamble advisories are suppressed like hook-env's. Without an active
# environment the command prints "No environment active!", but the advisories
# must not print either way.
# bats test_tags=hook:hook-env
@test "'flox deactivate' suppresses advisory preamble output" {
  _FLOX_FLOXHUB_GIT_URL="https://git.example.invalid/" \
    FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run --separate-stderr "$FLOX_BIN" deactivate
  assert_success
  refute_regex "$stderr" "as FloxHub host"
  refute_regex "$stderr" "token has expired"
}

# Pin that the suppression is scoped to the prompt-hook flow: user-invoked
# commands still print both advisories.
# bats test_tags=hook:hook-env
@test "user-invoked commands still print advisory preamble output" {
  _FLOX_FLOXHUB_GIT_URL="https://git.example.invalid/" \
    FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run --separate-stderr "$FLOX_BIN" config
  assert_success
  assert_regex "$stderr" "Using https://git.example.invalid/ as FloxHub host"
  assert_regex "$stderr" "Your FloxHub token has expired"
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

# bats test_tags=hook:deactivate:tcsh
@test "tcsh: interactive-type deactivate exits via the auto-fired prompt hook" {
  project_setup

  # tcsh's faulty-alias handling only applies to auto-fired special aliases:
  # an `exit` unwinding out of the eval'd `hook-env` output inside precmd makes
  # tcsh print "Faulty alias 'precmd' removed." and delete the alias WITHOUT
  # exiting the shell. The interactive deactivation script therefore sets
  # `_flox_exit` for the alias body to act on after the eval. Unlike the test
  # above, `precmd` must not be invoked manually here — a manual call is
  # ordinary alias expansion and bypasses the faulty-alias handling under test;
  # `tcsh -i` auto-fires precmd before each prompt even with stdin piped.
  #
  # The activation is in place; _FLOX_INVOCATION_TYPE is overridden to
  # `interactive` so the hook requests the interactive (exit) deactivation
  # script, as it would inside a real `flox activate` subshell.
  SESSION="$BATS_TEST_TMPDIR/interactive-deactivate.tcsh"
  cat > "$SESSION" <<EOF
eval "\`$FLOX_BIN activate -d $PROJECT_DIR\`"
setenv _FLOX_INVOCATION_TYPE interactive
$FLOX_BIN deactivate
echo SHOULD_NOT_PRINT: the shell exits at the next prompt, before this line
EOF

  run tcsh -i < "$SESSION"
  assert_success
  refute_output --partial "Faulty alias"
  refute_output --partial "SHOULD_NOT_PRINT"
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
