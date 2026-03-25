#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test flox hook, hook-env, and auto-activation deactivation
#
# bats file_tags=auto-activation
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# ---------------------------------------------------------------------------- #

# Helpers for project based tests.

project_setup() {
  export PROJECT_NAME="test"
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/$PROJECT_NAME"
  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" > /dev/null || return
  export MANIFEST_PATH="$PROJECT_DIR/.flox/env/manifest.toml"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

project_teardown() {
  popd > /dev/null || return
  rm -rf "${PROJECT_DIR?}"
  unset PROJECT_DIR
  unset MANIFEST_PATH
}

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  setup_isolated_flox
  project_setup

  # Clear any hook state from previous tests
  unset _FLOX_HOOK_DIFF
  unset _FLOX_HOOK_DIRS
  unset _FLOX_HOOK_WATCHES
  unset _FLOX_HOOK_SUPPRESSED
  unset _FLOX_HOOK_NOTIFIED
  unset _FLOX_HOOK_CWD
  unset _FLOX_HOOK_SAVE_PS1
}

teardown() {
  project_teardown
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# Hook command tests (no environment needed)
# ---------------------------------------------------------------------------- #

# bats test_tags=auto-activation:hook
@test "'flox hook bash' emits valid hook code" {
  run "$FLOX_BIN" hook bash
  assert_success
  assert_output --partial "_flox_hook"
  assert_output --partial "PROMPT_COMMAND"
  assert_output --partial "hook-env --shell bash"
  assert_output --partial "deactivate --shell bash"
}

# bats test_tags=auto-activation:hook
@test "'flox hook zsh' emits valid hook code" {
  run "$FLOX_BIN" hook zsh
  assert_success
  assert_output --partial "precmd_functions"
  assert_output --partial "chpwd_functions"
  assert_output --partial "hook-env --shell zsh"
}

# bats test_tags=auto-activation:hook
@test "'flox hook fish' emits valid hook code" {
  run "$FLOX_BIN" hook fish
  assert_success
  assert_output --partial "--on-event fish_prompt"
  assert_output --partial "--on-variable PWD"
  assert_output --partial "hook-env --shell fish"
}

# bats test_tags=auto-activation:hook
@test "'flox hook tcsh' emits valid hook code" {
  run "$FLOX_BIN" hook tcsh
  assert_success
  assert_output --partial "alias precmd"
  assert_output --partial "alias cwdcmd"
}

# bats test_tags=auto-activation:hook
@test "'flox hook' rejects unsupported shell" {
  run "$FLOX_BIN" hook powershell
  assert_failure
  assert_output --partial "unsupported shell"
}

# ---------------------------------------------------------------------------- #
# Hook-env tests
# ---------------------------------------------------------------------------- #

# bats test_tags=auto-activation:hook-env
@test "'hook-env' activates no environments when no .flox exists" {
  # Empty dir, no flox init
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  # hook-env emits state tracking vars even with no envs, but DIRS should be empty
  assert_output --partial "_FLOX_HOOK_DIRS=''"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' emits 'not trusted' warning for untrusted environment" {
  # Manually create .flox to bypass auto-trust from flox init
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "not trusted" "$BATS_TEST_TMPDIR/stderr"
  grep -q "flox trust" "$BATS_TEST_TMPDIR/stderr"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' emits state variables for trusted environment" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"
  assert_output --partial "_FLOX_HOOK_CWD"
  assert_output --partial "_FLOX_HOOK_DIFF"
  assert_output --partial "_FLOX_HOOK_WATCHES"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' sets prompt for trusted environment" {
  "$FLOX_BIN" init

  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "PS1"
  assert_output --partial "flox"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' produces no output on second call (fast path)" {
  # Use no environment to test the fast path without needing to eval
  # env vars that contaminate the test process (SSL_CERT_FILE, etc.)
  # First call — emits empty state vars for CWD tracking
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  # Verify first call produced output
  [ -n "$first_output" ]

  # Eval the state vars (safe — no env resolution, just empty state)
  eval "$first_output"

  # Second call — CWD unchanged, watches unchanged → fast path
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output ""
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' notifies about denied environments" {
  "$FLOX_BIN" init
  "$FLOX_BIN" trust --deny

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  local stderr_content
  stderr_content="$(cat "$BATS_TEST_TMPDIR/stderr")"

  # Denied environments should produce a "was denied" message
  [[ "$stderr_content" =~ "was denied" ]]
  [[ "$stderr_content" =~ "flox trust" ]]

  # hook-env still emits state vars, but DIRS should be empty (env not activated)
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS=''"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' detects deny after prior activation (no cd required)" {
  "$FLOX_BIN" init

  # First hook-env: activates the trusted environment
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$first_output" | grep '^export _FLOX_HOOK_')"

  # Verify the environment was activated
  [[ -n "$_FLOX_HOOK_DIRS" ]]

  # Deny the environment (simulates `flox trust --deny` while in the dir)
  "$FLOX_BIN" trust --deny

  # Next hook-env (same dir, no cd) should detect the trust change,
  # deactivate the env, and show the denied message.
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr_deny" || true
  local stderr_content
  stderr_content="$(cat "$BATS_TEST_TMPDIR/stderr_deny")"

  [[ "$stderr_content" =~ "was denied" ]]
  [[ "$stderr_content" =~ "flox trust" ]]
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' notifies about untrusted environment only once" {
  # Manually create .flox to bypass auto-trust
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  # First call — should warn about untrusted
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr1")" || true
  grep -q "not trusted" "$BATS_TEST_TMPDIR/stderr1"

  # Eval state vars so hook-env sees the notified list
  eval "$first_output"

  # Second call — should NOT warn again (already notified)
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr2" || true
  ! grep -q "not trusted" "$BATS_TEST_TMPDIR/stderr2"
}

# ---------------------------------------------------------------------------- #
# Deactivation tests
#
# Eval-ing hook-env output that resolves environments contaminates the test
# process with Nix store paths (e.g., SSL_CERT_FILE) that break httpmock.
# We run eval + subsequent commands in a subshell to avoid this.
# ---------------------------------------------------------------------------- #

# bats test_tags=auto-activation:deactivate
@test "'flox deactivate --shell bash' emits revert commands" {
  "$FLOX_BIN" init

  # Capture hook-env output and eval ONLY the _FLOX_HOOK_* state vars.
  # Eval-ing the full output would set Nix store paths (SSL_CERT_FILE, etc.)
  # that break httpmock in subsequent commands.
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep '^export _FLOX_HOOK_')"

  # Verify we have active hook state
  [ -n "$_FLOX_HOOK_DIRS" ]

  run "$FLOX_BIN" deactivate --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_SUPPRESSED"
  assert_output --partial "unset _FLOX_HOOK_DIFF"
  assert_output --partial "unset _FLOX_HOOK_DIRS"
  assert_output --partial "unset _FLOX_HOOK_WATCHES"
}

# bats test_tags=auto-activation:deactivate
@test "deactivate prevents re-activation on next hook-env" {
  "$FLOX_BIN" init

  # Eval only _FLOX_HOOK_* state vars to avoid SSL env contamination
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep '^export _FLOX_HOOK_')"

  # Deactivate — also eval only state vars
  local deactivate_output
  deactivate_output="$("$FLOX_BIN" deactivate --shell bash 2>/dev/null)"
  eval "$(echo "$deactivate_output" | grep -E '^(export _FLOX_HOOK_|unset _FLOX_HOOK_)')"

  # Verify _FLOX_HOOK_SUPPRESSED is set
  [ -n "$_FLOX_HOOK_SUPPRESSED" ]

  # Next hook-env should NOT re-activate the suppressed environment.
  # hook-env detects nothing changed (dirs still empty) and produces no output.
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output ""
}

# ---------------------------------------------------------------------------- #
# Composition tests
# ---------------------------------------------------------------------------- #

# bats test_tags=auto-activation:composition
@test "multiple nested environments both activate" {
  # Create outer env
  mkdir -p outer
  pushd outer > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  # Create inner env
  mkdir -p outer/inner
  pushd outer/inner > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  # cd to inner, run hook-env
  pushd outer/inner > /dev/null
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"

  # The _FLOX_HOOK_DIRS should contain both paths (colon-separated)
  local dirs_line
  dirs_line="$(echo "$output" | grep "_FLOX_HOOK_DIRS")"
  # Should contain both outer and inner .flox paths
  [[ "$dirs_line" =~ "outer" ]]
  popd > /dev/null
}

# bats test_tags=auto-activation:composition
@test "switching directories changes active environment" {
  # Init env A
  mkdir -p projA
  pushd projA > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  # Init env B
  mkdir -p projB
  pushd projB > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  # Activate in projA — eval only _FLOX_HOOK_* state vars
  pushd projA > /dev/null
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep '^export _FLOX_HOOK_')"
  popd > /dev/null

  # Switch to projB
  pushd projB > /dev/null
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  # Should emit new state (dirs changed)
  assert_output --partial "_FLOX_HOOK_DIRS"
  # The DIRS should reference projB, not projA
  local dirs_line
  dirs_line="$(echo "$output" | grep "_FLOX_HOOK_DIRS")"
  [[ "$dirs_line" =~ "projB" ]]
  [[ ! "$dirs_line" =~ "projA" ]]
  popd > /dev/null
}
