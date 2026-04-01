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
  # No envs discovered and no dirs changed, so only CWD tracking is emitted.
  # DIRS is not emitted (unchanged from its default empty state).
  refute_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' prompts for unregistered environment" {
  # Manually create .flox to bypass auto-trust from flox init
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' emits state variables for enabled environment" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"
  assert_output --partial "_FLOX_HOOK_CWD"
  assert_output --partial "_FLOX_HOOK_DIFF"
  assert_output --partial "_FLOX_HOOK_WATCHES"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' sets prompt for enabled environment" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

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
@test "'hook-env' notifies about disabled environments" {
  "$FLOX_BIN" init
  "$FLOX_BIN" disable

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  local stderr_content
  stderr_content="$(cat "$BATS_TEST_TMPDIR/stderr")"

  # Disabled environments should produce a "disabled" message
  [[ "$stderr_content" =~ "disabled" ]]
  [[ "$stderr_content" =~ "flox enable" ]]

  # Second call (without eval'ing first call's state) — disabled env is not
  # activated, so DIRS remains unchanged (not emitted).  Only CWD and the
  # updated NOTIFIED list are emitted.
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  refute_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' detects disable after prior activation (no cd required)" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # First hook-env: activates the enabled environment
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$first_output" | grep '^export _FLOX_HOOK_')"

  # Verify the environment was activated
  [[ -n "$_FLOX_HOOK_DIRS" ]]

  # Disable the environment (simulates `flox disable` while in the dir)
  "$FLOX_BIN" disable

  # Next hook-env (same dir, no cd) should detect the preference change,
  # deactivate the env, and show the disabled message.
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr_disable" || true
  local stderr_content
  stderr_content="$(cat "$BATS_TEST_TMPDIR/stderr_disable")"

  [[ "$stderr_content" =~ "disabled" ]]
  [[ "$stderr_content" =~ "flox enable" ]]
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' notifies about unregistered environment only once" {
  # Manually create .flox to bypass auto-trust
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  # First call — should suggest flox enable
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr1")" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr1"

  # Eval state vars so hook-env sees the notified list
  eval "$first_output"

  # Second call — should NOT warn again (already notified)
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr2" || true
  ! grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr2"
}

# bats test_tags=auto-activation:hook-env
@test "'flox init' does NOT auto-enable auto-activation" {
  "$FLOX_BIN" init

  # Without 'flox enable', hook-env should not activate the environment.
  # It should suggest running 'flox enable'.
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr"
  # DIRS should not be emitted (env not activated)
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  refute_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "'flox enable' enables auto-activation and sets trust for local envs" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "'flox enable' then 'flox disable' toggles correctly" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Verify it activates
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"

  # Disable
  "$FLOX_BIN" disable

  # Should no longer activate
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "disabled" "$BATS_TEST_TMPDIR/stderr"
}

# bats test_tags=auto-activation:hook-env
@test "environment with trust but no preference does NOT auto-activate" {
  # Manually create .flox and trust it, but don't enable preference
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  # This would only set trust, not preference (old behavior)
  # We can't easily call TrustManager directly from bats, but flox init
  # sets trust. So just verify that init + no enable = no activation.
  "$FLOX_BIN" init

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr"
}

# bats test_tags=auto-activation:hook-env
@test "preference persists across manifest changes (not content-sensitive)" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Verify it activates
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$first_output" | grep '^export _FLOX_HOOK_')"
  [[ -n "$_FLOX_HOOK_DIRS" ]]

  # Modify the manifest
  echo "" >> "$MANIFEST_PATH"

  # Preference should still be enabled (not content-sensitive)
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  # Should still emit DIRS (environment still enabled)
  assert_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "auto_activate 'always' skips prompt for unregistered environments" {
  "$FLOX_BIN" init
  # Set config to always auto-activate
  "$FLOX_BIN" config --set auto_activate '"always"'

  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"
}

# bats test_tags=auto-activation:hook-env
@test "auto_activate 'never' disables all auto-activation globally" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Set config to never auto-activate
  "$FLOX_BIN" config --set auto_activate '"never"'

  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr" || true
  grep -q "disabled globally" "$BATS_TEST_TMPDIR/stderr"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' shows notice after decline and cd away/back" {
  "$FLOX_BIN" init
  "$FLOX_BIN" disable  # Simulates what interactive "N" now does

  # First call — notice shown
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr1")" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr1"
  eval "$first_output"

  # cd away
  pushd .. > /dev/null
  local away_output
  away_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)" || true
  eval "$away_output"
  popd > /dev/null

  # cd back — notice should re-appear (informational, not interactive)
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr_back" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr_back"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' does not double-prompt on sequential calls (chpwd+precmd)" {
  mkdir -p .flox/env
  echo '{"name":"test","version":1}' > .flox/env.json
  with_latest_schema > .flox/env/manifest.toml

  # First call — prompts (non-interactive → auto-decline)
  local first_output
  first_output="$("$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr1")" || true
  eval "$first_output"

  # Second call (simulating precmd after chpwd) — should NOT prompt/notify
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr2" || true
  ! grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr2"
}

# bats test_tags=auto-activation:hook-env
@test "'hook-env' decline persists across sessions" {
  "$FLOX_BIN" init
  "$FLOX_BIN" disable  # Simulates interactive "N" persistence

  # First "session"
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr1" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr1"

  # Clear all hook state to simulate new session
  unset _FLOX_HOOK_DIFF _FLOX_HOOK_DIRS _FLOX_HOOK_WATCHES
  unset _FLOX_HOOK_SUPPRESSED _FLOX_HOOK_NOTIFIED _FLOX_HOOK_CWD
  unset _FLOX_HOOK_ACTIVATIONS

  # New "session" — should show notice (not prompt), because disable is persisted to disk
  "$FLOX_BIN" hook-env --shell bash 2>"$BATS_TEST_TMPDIR/stderr2" || true
  grep -q "flox enable" "$BATS_TEST_TMPDIR/stderr2"
}

# bats test_tags=auto-activation:hook-env
@test "'flox enable --path' works for non-CWD environments" {
  # Create env in a subdirectory
  mkdir -p subdir
  pushd subdir > /dev/null
  "$FLOX_BIN" init
  popd > /dev/null

  # Enable from parent directory using --path
  "$FLOX_BIN" enable --path subdir

  # cd to subdir and verify it activates
  pushd subdir > /dev/null
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "_FLOX_HOOK_DIRS"
  popd > /dev/null
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
  "$FLOX_BIN" enable

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
  "$FLOX_BIN" enable

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

# bats test_tags=auto-activation:deactivate
@test "deactivate clears _FLOX_ACTIVE_ENVIRONMENTS" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Activate via hook-env — eval only _FLOX_HOOK_* state vars
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep -E '^export (_FLOX_HOOK_|_FLOX_ACTIVE_ENVIRONMENTS)')"

  # Verify env is in _FLOX_ACTIVE_ENVIRONMENTS
  [ -n "$_FLOX_ACTIVE_ENVIRONMENTS" ]

  # Deactivate — eval the output
  local deactivate_output
  deactivate_output="$("$FLOX_BIN" deactivate --shell bash 2>/dev/null)"
  eval "$(echo "$deactivate_output" | grep -E '^(export _FLOX_|unset _FLOX_)')"

  # _FLOX_ACTIVE_ENVIRONMENTS should no longer contain the environment
  # (it should be empty or not contain the project's .flox path)
  [[ "$_FLOX_ACTIVE_ENVIRONMENTS" != *"$PROJECT_DIR"* ]]
}

# bats test_tags=auto-activation:deactivate
@test "deactivate in nested hierarchy only suppresses innermost" {
  # Create outer env
  mkdir -p outer
  pushd outer > /dev/null
  "$FLOX_BIN" init
  "$FLOX_BIN" enable
  popd > /dev/null

  # Create inner env
  mkdir -p outer/inner
  pushd outer/inner > /dev/null
  "$FLOX_BIN" init
  "$FLOX_BIN" enable
  popd > /dev/null

  # cd to inner, run hook-env to activate both
  pushd outer/inner > /dev/null
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep -E '^export (_FLOX_HOOK_|_FLOX_ACTIVE_ENVIRONMENTS)')"

  # Verify both dirs are active
  [[ "$_FLOX_HOOK_DIRS" =~ "outer" ]]

  # Deactivate — should only suppress the innermost (inner)
  local deactivate_output
  deactivate_output="$("$FLOX_BIN" deactivate --shell bash 2>/dev/null)"
  eval "$(echo "$deactivate_output" | grep -E '^(export _FLOX_|unset _FLOX_)')"

  # _FLOX_HOOK_SUPPRESSED should contain only the inner .flox path
  [[ "$_FLOX_HOOK_SUPPRESSED" =~ "inner" ]]

  # Next hook-env should re-activate outer but not inner
  local hook_output2
  hook_output2="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  local dirs_line
  dirs_line="$(echo "$hook_output2" | grep "_FLOX_HOOK_DIRS")"
  [[ "$dirs_line" =~ "outer" ]]
  [[ ! "$dirs_line" =~ "inner" ]]
  popd > /dev/null
}

# bats test_tags=auto-activation:deactivate
@test "flox activate works after flox deactivate" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Activate via hook-env
  local hook_output
  hook_output="$("$FLOX_BIN" hook-env --shell bash 2>/dev/null)"
  eval "$(echo "$hook_output" | grep -E '^export (_FLOX_HOOK_|_FLOX_ACTIVE_ENVIRONMENTS)')"

  # Deactivate
  local deactivate_output
  deactivate_output="$("$FLOX_BIN" deactivate --shell bash 2>/dev/null)"
  eval "$(echo "$deactivate_output" | grep -E '^(export _FLOX_|unset _FLOX_)')"

  # flox activate should NOT error with "already active"
  run "$FLOX_BIN" activate -- echo "activated successfully"
  assert_success
  assert_output --partial "activated successfully"
}

# bats test_tags=auto-activation:deactivate
@test "hook-env emits prompt on first call in subshell with exclude vars" {
  "$FLOX_BIN" init
  "$FLOX_BIN" enable

  # Simulate the subshell state after `flox activate` spawns a subshell
  # for a project env: hook state is cleared, but exclude vars are set.
  local dot_flox_path
  dot_flox_path="$(pwd)/.flox"
  local env_name
  env_name="$(basename "$(pwd)")"

  # Clear hook state (as clear_hook_state does)
  unset _FLOX_HOOK_DIFF _FLOX_HOOK_DIRS _FLOX_HOOK_WATCHES
  unset _FLOX_HOOK_SUPPRESSED _FLOX_HOOK_NOTIFIED _FLOX_HOOK_CWD
  unset _FLOX_HOOK_ACTIVATIONS _FLOX_HOOK_SAVE_PS1

  # Set exclude vars (as set_hook_exclude_vars does)
  export _FLOX_HOOK_EXCLUDE_DIRS="$dot_flox_path"
  export _FLOX_HOOK_EXCLUDE_NAMES="$env_name"

  # First hook-env call should emit prompt-setting code
  run "$FLOX_BIN" hook-env --shell bash
  assert_success
  assert_output --partial "PS1="
  assert_output --partial "$env_name"
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
  "$FLOX_BIN" enable
  popd > /dev/null

  # Create inner env
  mkdir -p outer/inner
  pushd outer/inner > /dev/null
  "$FLOX_BIN" init
  "$FLOX_BIN" enable
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
  "$FLOX_BIN" enable
  popd > /dev/null

  # Init env B
  mkdir -p projB
  pushd projB > /dev/null
  "$FLOX_BIN" init
  "$FLOX_BIN" enable
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
