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

# Set a `[vars]` manifest for the project at $1 defining $2 = "$3".
set_vars_manifest() {
  cat << EOF | "$FLOX_BIN" edit -d "$1" -f -
version = 1

[vars]
$2 = "$3"
EOF
}

# A second project with an observable var, as a target for auto-activation.
project2_setup() {
  export PROJECT2_DIR="${BATS_TEST_TMPDIR?}/project2-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECT2_DIR"
  mkdir -p "$PROJECT2_DIR"
  "$FLOX_BIN" init -d "$PROJECT2_DIR"
  set_vars_manifest "$PROJECT2_DIR" TEST_VAR2 auto2
}

project2_teardown() {
  rm -rf "${PROJECT2_DIR?}"
  unset PROJECT2_DIR
}

# A project nested inside PROJECT2_DIR, for stacked auto-activation.
# Cleaned up by project2_teardown.
project3_setup() {
  export PROJECT3_DIR="${PROJECT2_DIR?}/nested"
  mkdir -p "$PROJECT3_DIR"
  "$FLOX_BIN" init -d "$PROJECT3_DIR"
  set_vars_manifest "$PROJECT3_DIR" TEST_VAR3 auto3
}

# A sibling project with an observable var, as a target for switching away
# from a nested stack.
projectz_setup() {
  export PROJECTZ_DIR="${BATS_TEST_TMPDIR?}/projectz-${BATS_TEST_NUMBER?}"
  rm -rf "$PROJECTZ_DIR"
  mkdir -p "$PROJECTZ_DIR"
  "$FLOX_BIN" init -d "$PROJECTZ_DIR"
  set_vars_manifest "$PROJECTZ_DIR" TEST_VARZ autoz
}

projectz_teardown() {
  rm -rf "${PROJECTZ_DIR?}"
  unset PROJECTZ_DIR
}

setup() {
  common_test_setup
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  if [ -n "${PROJECTZ_DIR:-}" ]; then
    projectz_teardown
  fi
  if [ -n "${PROJECT2_DIR:-}" ]; then
    project2_teardown
  fi
  if [ -n "${PROJECT_DIR:-}" ]; then
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #
# hook-env: feature flag gating
# ---------------------------------------------------------------------------- #

# Deactivate-action handling in hook-env is not gated, but the auto-activation
# logic still is: without the flag the hook emits nothing, even with a
# discoverable environment in the working directory.
# TODO: Remove this test when the auto_activate feature flag is removed.
# bats test_tags=hook:hook-env
@test "'flox hook-env' succeeds without auto_activate feature flag but doesn't auto-activate" {
  project_setup
  unset FLOX_FEATURES_AUTO_ACTIVATE
  run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$" --invocation-type inplace
  assert_success
  assert_output ""
}

# ---------------------------------------------------------------------------- #
# Auto-activation: cd into a project activates its environment
# ---------------------------------------------------------------------------- #
#
# Each test activates PROJECT_DIR to install the prompt hook, then cd's into
# a second project and fires the hook manually to stand in for the next
# prompt. The hook should activate the second project's environment in place.
#
# Each test has the shell call `flox activate` directly (not pre-captured in
# a bats variable) to avoid quoting issues across shells.

# bats test_tags=hook:auto-activate:bash
@test "bash: hook auto-activates a discovered environment on cd" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"var2:\$TEST_VAR2\"
    printenv _FLOX_AUTO_ACTIVATED_ENVIRONMENTS
  "
  assert_success
  assert_output --partial "var2:auto2"
  assert_output --partial "$(realpath "$PROJECT2_DIR")"
}

# bats test_tags=hook:auto-activate:zsh
@test "zsh: hook auto-activates a discovered environment on cd" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr zsh -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which zsh)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"var2:\$TEST_VAR2\"
    printenv _FLOX_AUTO_ACTIVATED_ENVIRONMENTS
  "
  assert_success
  assert_output --partial "var2:auto2"
  assert_output --partial "$(realpath "$PROJECT2_DIR")"
}

# bats test_tags=hook:auto-activate:fish
@test "fish: hook auto-activates a discovered environment on cd" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr fish -c "
    set -gx FLOX_FEATURES_AUTO_ACTIVATE true
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    cd $PROJECT2_DIR
    _flox_hook
    echo \"var2:\$TEST_VAR2\"
    printenv _FLOX_AUTO_ACTIVATED_ENVIRONMENTS
  "
  assert_success
  assert_output --partial "var2:auto2"
  assert_output --partial "$(realpath "$PROJECT2_DIR")"
}

# bats test_tags=hook:auto-activate:tcsh
@test "tcsh: hook auto-activates a discovered environment on cd" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr tcsh -c "
    setenv FLOX_FEATURES_AUTO_ACTIVATE true
    eval \"\`$FLOX_BIN activate -d $PROJECT_DIR\`\"
    cd $PROJECT2_DIR
    precmd
    echo \"var2:\$TEST_VAR2\"
    printenv _FLOX_AUTO_ACTIVATED_ENVIRONMENTS
  "
  assert_success
  assert_output --partial "var2:auto2"
  assert_output --partial "$(realpath "$PROJECT2_DIR")"
}

# ---------------------------------------------------------------------------- #
# Auto-deactivation: leaving the project directory deactivates the environment
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-deactivate:bash
@test "bash: auto-activated environment deactivates after leaving its directory" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"during:\$TEST_VAR2\"
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"after:\${TEST_VAR2:-unset}\"
    echo \"tracked:\${_FLOX_AUTO_ACTIVATED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  assert_output --partial "during:auto2"
  assert_output --partial "after:unset"
  assert_output --partial "tracked:unset"
}

# bats test_tags=hook:auto-deactivate:manual
@test "bash: manually activated environment is not deactivated on leaving" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  set_vars_manifest "$PROJECT_DIR" TEST_VAR manual

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"after:\${TEST_VAR:-unset}\"
  "
  assert_success
  assert_output --partial "after:manual"
}

# bats test_tags=hook:auto-activate:reentry
@test "bash: re-entering a project after leaving re-activates it" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"out:\${TEST_VAR2:-unset}\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"back:\$TEST_VAR2\"
  "
  assert_success
  assert_output --partial "out:unset"
  assert_output --partial "back:auto2"
}

# ---------------------------------------------------------------------------- #
# Stacked auto-activation: nested projects activate outermost-first and the
# whole stack unwinds in a single hook run on leaving (DEV-111)
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-activate:nested
@test "bash: nested projects activate as a stack and unwind together on leaving" {
  project_setup
  project2_setup
  project3_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT3_DIR
    _flox_hook
    echo \"in: v2:\$TEST_VAR2 v3:\$TEST_VAR3\"
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"out: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"
    echo \"tracked:\${_FLOX_AUTO_ACTIVATED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  assert_output --partial "in: v2:auto2 v3:auto3"
  # Both layers pop in the single hook run after leaving.
  assert_output --partial "out: v2:unset v3:unset"
  assert_output --partial "tracked:unset"
}

# ---------------------------------------------------------------------------- #
# Suppression: 'flox deactivate' inside the project must not be undone by
# the next prompt
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:suppress:bash
@test "bash: 'flox deactivate' suppresses re-activation until the directory is left" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"during:\$TEST_VAR2\"
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after:\${TEST_VAR2:-unset}\"
    _flox_hook
    echo \"still:\${TEST_VAR2:-unset}\"
    printenv _FLOX_SUPPRESSED_ENVIRONMENTS
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"left:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"back:\$TEST_VAR2\"
  "
  assert_success
  assert_output --partial "during:auto2"
  assert_output --partial "after:unset"
  assert_output --partial "still:unset"
  assert_output --partial "$(realpath "$PROJECT2_DIR")"
  # Leaving the directory revokes the suppression; re-entering re-activates.
  assert_output --partial "left:unset"
  assert_output --partial "back:auto2"
}

# ---------------------------------------------------------------------------- #
# Hook auto-fires: verify the prompt hook triggers without manual invocation
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-fires
@test "bash: hook auto-activates via PROMPT_COMMAND in interactive shell" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-cd.exp" "$PROJECT_DIR" "$PROJECT2_DIR" 'echo TEST_VAR2="$TEST_VAR2"'
  assert_output --partial 'TEST_VAR2=auto2'
}

# bats test_tags=hook:auto-fires:nested
@test "bash: interactive hook unwinds a nested stack and switches projects in one prompt" {
  project_setup
  project2_setup
  project3_setup
  projectz_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-nested-cd.exp" "$PROJECT_DIR" "$PROJECT3_DIR" "$PROJECTZ_DIR"
  # The whole stack pops in the prompt run that switches projects, so no
  # layer is buried and abandoned ...
  refute_output --partial "Did not auto-deactivate"
  # ... and the consecutive deactivations must not leave tracer noise from
  # re-sourcing set-prompt after a previous pop unset the tracer.
  refute_output --partial "command not found"
}

# ---------------------------------------------------------------------------- #
# Plain `flox deactivate` is serviced by the prompt hook
# ---------------------------------------------------------------------------- #
#
# `flox deactivate` (no `--print-script`) writes a prompt-hook action file; the
# next time the prompt hook runs `flox hook-env`, it restores the environment in
# place. We fire `_flox_hook` manually to stand in for the next prompt.

set_test_var_manifest() {
  set_vars_manifest "$PROJECT_DIR" TEST_VAR modified
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
