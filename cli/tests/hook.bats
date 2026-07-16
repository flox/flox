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

# A three-level nested chain outer ⊃ mid ⊃ inner, each with a distinct env
# name (so `FLOX_PROMPT_ENVIRONMENTS` order is observable) and an observable
# var. Used to exercise mid-stack re-insertion. Cleaned up by nested_chain_teardown.
nested_chain_setup() {
  # Use a fixed basename for each level (inside the per-test tmpdir) so the env
  # names — and thus FLOX_PROMPT_ENVIRONMENTS — are stable and assertable.
  export NEST_ROOT_DIR="${BATS_TEST_TMPDIR?}/nest-${BATS_TEST_NUMBER?}"
  export NEST_OUTER_DIR="${NEST_ROOT_DIR?}/outer"
  export NEST_MID_DIR="${NEST_OUTER_DIR?}/mid"
  export NEST_INNER_DIR="${NEST_MID_DIR?}/inner"
  rm -rf "$NEST_ROOT_DIR"
  mkdir -p "$NEST_INNER_DIR"
  "$FLOX_BIN" init -d "$NEST_OUTER_DIR"
  "$FLOX_BIN" init -d "$NEST_MID_DIR"
  "$FLOX_BIN" init -d "$NEST_INNER_DIR"
  set_vars_manifest "$NEST_OUTER_DIR" TEST_OUTER outer
  set_vars_manifest "$NEST_MID_DIR" TEST_MID mid
  set_vars_manifest "$NEST_INNER_DIR" TEST_INNER inner
}

nested_chain_teardown() {
  rm -rf "${NEST_ROOT_DIR?}"
  unset NEST_ROOT_DIR NEST_OUTER_DIR NEST_MID_DIR NEST_INNER_DIR
}

setup_file() {
  common_file_setup
  # Many of these tests drive real, interactive `flox activate` runs through
  # `expect`, and several build and activate two or three environments in a
  # single test. Run in parallel within the file, they contend on the shared
  # Nix store and pile up concurrent executive (activation daemon) processes,
  # which can starve an activation long enough to blow past the `expect`
  # timeout — a ~60s window with no terminal output that surfaces as a flaky
  # timeout on whichever consent/activation test loses the race. Serialize the
  # file so these interactive tests stay deterministic. Mirrors
  # containerize.bats, which serializes for the same reason.
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true
}

setup() {
  common_test_setup
  # This file exercises `prompt`-mode auto-activation (consent prompts and the
  # default first-encounter behaviour) and the prompt hook itself, so override
  # the suite-wide `disabled`/`disable_hook = true` defaults for every test
  # here.
  export FLOX_AUTO_ACTIVATE=prompt
  enable_prompt_hook
  setup_isolated_flox
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/empty.yaml"
}

teardown() {
  if [ -n "${NEST_ROOT_DIR:-}" ]; then
    nested_chain_teardown
  fi
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
  # An empty cwd keeps this hermetic: a `.flox` littered into the shared bats
  # cwd by an unrelated test would give hook-env auto-activation work and trip
  # its stale-prompt-hook check instead of exercising the advisory suppression.
  cd "$BATS_TEST_TMPDIR"
  _FLOX_FLOXHUB_GIT_URL="https://git.example.invalid/" \
    FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$"
  assert_success
  assert_equal "$stderr" ""
  assert_output ""
}

# An invalid token is normally surfaced and removed from the user's config;
# both must be deferred to the next user-invoked command rather than printing
# and rewriting config on every prompt.
# bats test_tags=hook:hook-env
@test "'flox hook-env' defers invalid token cleanup to user-invoked commands" {
  # See the empty-cwd note in the first hook-env test.
  cd "$BATS_TEST_TMPDIR"
  # The suite-wide env token would shadow the invalid file token (env wins over
  # the user config file), so drop it for this test.
  unset FLOX_FLOXHUB_TOKEN
  mkdir -p "$FLOX_CONFIG_DIR"
  echo 'floxhub_token = "not-a-jwt"' >> "$FLOX_CONFIG_DIR/flox.toml"

  run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$"
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
  refute_regex "$stderr" "as the FloxHub git endpoint"
  refute_regex "$stderr" "not logged in to FloxHub"
}

# Pin that the suppression is scoped to the prompt-hook flow: user-invoked
# commands still print both advisories.
# bats test_tags=hook:hook-env
@test "user-invoked commands still print advisory preamble output" {
  _FLOX_FLOXHUB_GIT_URL="https://git.example.invalid/" \
    FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run --separate-stderr "$FLOX_BIN" config
  assert_success
  assert_regex "$stderr" "Using https://git.example.invalid/ as the FloxHub git endpoint"
  assert_regex "$stderr" "You are not logged in to FloxHub"
}

# The logged-out reminder is account-global, so a single user action that
# nests `flox` invocations — e.g. `flox activate` whose shell rc runs
# `flox activate` again — should surface it only once. The outermost activation
# warns; anything already inside an activation stays quiet.
# bats test_tags=hook:hook-env
@test "logged-out advisory is shown once across nested 'flox' invocations" {
  project_setup

  FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run "$FLOX_BIN" activate -d "$PROJECT_DIR" -- \
    "$FLOX_BIN" activate -d "$PROJECT_DIR" -- true
  assert_success
  # Once for the outer activation, and not again for the nested one.
  run grep -c "You are not logged in to FloxHub" <<< "$output"
  assert_output "1"

  project_teardown
}

# Mirrors the real-world trigger: a shell rc activates an environment in place
# (`eval "$(flox activate)"`), then the user runs another `flox` command in the
# same shell. The in-place activation exports `_FLOX_ACTIVE_ENVIRONMENTS`, so the
# second command sees it is nested and does not repeat the reminder.
# bats test_tags=hook:hook-env
@test "logged-out advisory is not repeated after an in-place activation" {
  project_setup

  FLOX_FLOXHUB_TOKEN="$EXPIRED_FLOXHUB_TOKEN" \
    run bash -c "eval \"\$('$FLOX_BIN' activate -d '$PROJECT_DIR')\"; '$FLOX_BIN' config"
  assert_success
  # Once for the in-place activation, and not again for the later command.
  run grep -c "You are not logged in to FloxHub" <<< "$output"
  assert_output "1"

  project_teardown
}

# A user who never logged in gets the same reminder as one whose token expired.
# bats test_tags=hook:hook-env
@test "user-invoked commands warn when not logged in" {
  unset FLOX_FLOXHUB_TOKEN
  run --separate-stderr "$FLOX_BIN" config
  assert_success
  assert_regex "$stderr" "You are not logged in to FloxHub. Run 'flox auth login' to log in."
}

# bats test_tags=hook:hook-env
@test "'flox hook-env' suppresses the logged-out advisory when no token is set" {
  # See the empty-cwd note in the first hook-env test.
  cd "$BATS_TEST_TMPDIR"
  unset FLOX_FLOXHUB_TOKEN
  run --separate-stderr "$FLOX_BIN" hook-env --shell bash --shell-pid "$$"
  assert_success
  assert_equal "$stderr" ""
  assert_output ""
}

# `flox auth` subcommands either log the user in or already report the
# logged-out state themselves, so the reminder must not be stacked on top.
# bats test_tags=hook:hook-env
@test "'flox auth' subcommands do not repeat the logged-out advisory" {
  unset FLOX_FLOXHUB_TOKEN
  run "$FLOX_BIN" auth status
  assert_output --partial "You are not currently logged in to FloxHub."
  refute_output --partial "Run 'flox auth login'"
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr zsh -c "
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr fish -c "
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

# bats test_tags=hook:auto-activate:fish
@test "fish: hook auto-activation sets the prompt" {
  project_setup
  project2_setup
  # Mirror the reported scenario: the hook-registering outer activation is a
  # 'default' env hidden from FLOX_PROMPT_ENVIRONMENTS, so the outer
  # activation defines no prompt variables at the top level. (An unscoped fish
  # `set` reuses an existing variable's scope, so a visible outer env would
  # mask scoping bugs when the hook later sets prompt variables inside a
  # function.) Hiding is opt-in, so pin the config the scenario relies on.
  "$FLOX_BIN" config --set hide_default_prompt true
  "$FLOX_BIN" edit -d "$PROJECT_DIR" --name default
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  # unbuffer provides a pty so that `isatty 1` holds and set-prompt.fish is
  # sourced. NO_COLOR keeps the prompt prefix a plain "flox [<envs>]" string.
  # A known fish_prompt is defined up front because non-interactive fish has
  # no prompt function for set-prompt.fish to save and wrap.
  run unbuffer fish -c '
    set -gx NO_COLOR 1
    function fish_prompt; echo -n "knownPrompt> "; end
    eval ("$FLOX_BIN" activate -d "$PROJECT_DIR")
    cd "$PROJECT2_DIR"
    _flox_hook
    echo "<prompt>"(fish_prompt)"</prompt>"
  '
  assert_success
  assert_output --partial "<prompt>flox [project2-"
  assert_output --partial "knownPrompt>"
}

# bats test_tags=hook:auto-activate:tcsh
@test "tcsh: hook auto-activates a discovered environment on cd" {
  project_setup
  project2_setup
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr tcsh -c "
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
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

# An out-of-sync prompt hook (registered by an incompatible version of Flox,
# or with a lost marker) can't trust the diffs the sweep would decode, so
# `hook-env` errors instead of doing any auto-activation work. `_flox_hook`
# preserves the previous command's exit code by design, so the sweep is
# invoked directly to observe the failure.
# bats test_tags=hook:auto-deactivate:stale-version
@test "bash: stale prompt-hook version fails the sweep" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"during:\$TEST_VAR2\"
    cd $BATS_TEST_TMPDIR
    export _FLOX_PROMPT_HOOK_VERSION=99:true
    $FLOX_BIN hook-env --shell bash --shell-pid \$\$
  "
  assert_failure
  assert_output --partial "during:auto2"
  assert_output --partial "out of sync with the running Flox"
}

# The stale-version guard errors only when there is auto-activation work; a
# shell sitting inside its active environment with nothing to do stays quiet.
# bats test_tags=hook:auto-deactivate:stale-version-idle
@test "bash: stale prompt-hook version stays quiet with nothing to auto-activate" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    export _FLOX_PROMPT_HOOK_VERSION=99:true
    _flox_hook
    echo done
  "
  assert_success
  assert_output --partial "done"
  refute_output --partial "out of sync"
}

# Deleting a project directory while its environment is auto-activated (e.g.
# removing a git worktree with tooling that cd's back to the main checkout)
# must still tear the environment down; the deactivation data is recovered
# from the activation state rather than the deleted directory.
# bats test_tags=hook:auto-deactivate:deleted-dir
@test "bash: auto-deactivates an environment whose directory was deleted" {
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"during:\$TEST_VAR2\"
    cd $PROJECT_DIR
    rm -rf $PROJECT2_DIR
    _flox_hook
    echo \"after:\${TEST_VAR2:-unset}\"
    echo \"tracked:\${_FLOX_AUTO_ACTIVATED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  assert_output --partial "during:auto2"
  assert_output --partial "after:unset"
  assert_output --partial "tracked:unset"
  refute_regex "$stderr" "Did not find an environment"
}

# bats test_tags=hook:auto-deactivate:manual
@test "bash: manually activated environment is not deactivated on leaving" {
  project_setup
  set_vars_manifest "$PROJECT_DIR" TEST_VAR manual

  run --separate-stderr bash -c "
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
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
# whole stack unwinds in a single hook run on leaving
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-activate:nested
@test "bash: nested projects activate as a stack and unwind together on leaving" {
  project_setup
  project2_setup
  project3_setup
  # Auto-activation is opt-in; allow both layers of the nested stack.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"

  run --separate-stderr bash -c "
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
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
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
# Config: 'flox activate deny' suppresses auto-activation for a directory
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:deny:bash
@test "bash: hook does not auto-activate an environment denied via 'flox activate deny'" {
  project_setup
  project2_setup

  # Record the deny preference for the second project before entering it.
  "$FLOX_BIN" activate deny -d "$PROJECT2_DIR"

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"var2:\${TEST_VAR2:-unset}\"
    echo \"tracked:\${_FLOX_AUTO_ACTIVATED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  # The denied environment is neither activated nor tracked.
  assert_output --partial "var2:unset"
  assert_output --partial "tracked:unset"
}

# ---------------------------------------------------------------------------- #
# Config: 'prompt' mode (the default) asks for consent before auto-activating
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:prompt:default
@test "bash: unregistered environment is not auto-activated without consent" {
  project_setup
  project2_setup
  # No allow/deny recorded, so the default 'prompt' mode applies. This run is
  # non-interactive (no controlling terminal), so the hook cannot prompt and
  # must leave the environment unregistered rather than auto-activating it.

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"var2:\${TEST_VAR2:-unset}\"
    echo \"tracked:\${_FLOX_AUTO_ACTIVATED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  assert_output --partial "var2:unset"
  assert_output --partial "tracked:unset"
}

# bats test_tags=hook:prompt:yes
@test "bash: answering the consent prompt with 'y' auto-activates the environment" {
  project_setup
  project2_setup

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-consent.exp" "$PROJECT_DIR" "$PROJECT2_DIR" "y" 'echo TEST_VAR2="$TEST_VAR2"'
  assert_output --partial 'TEST_VAR2=auto2'
}

# bats test_tags=hook:prompt:no
@test "bash: declining the consent prompt does not auto-activate the environment" {
  project_setup
  project2_setup

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-consent.exp" "$PROJECT_DIR" "$PROJECT2_DIR" "n" 'echo TEST_VAR2="$TEST_VAR2"'
  refute_output --partial 'TEST_VAR2=auto2'
}

# bats test_tags=hook:prompt:batched
@test "bash: a single consent prompt activates a whole nested hierarchy" {
  project_setup
  project2_setup
  project3_setup

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  # Entering the nested project discovers two unregistered environments
  # (PROJECT2 and the nested PROJECT3). A single 'y' must activate both.
  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-consent.exp" "$PROJECT_DIR" "$PROJECT3_DIR" "y" 'echo "v2:$TEST_VAR2 v3:$TEST_VAR3"'
  assert_output --partial 'v2:auto2 v3:auto3'
  # A single batched prompt covered both environments.
  assert_output --partial 'Auto-activate these 2 environments'
}

# bats test_tags=hook:prompt:batched:no
@test "bash: declining a batched consent prompt shows a single note" {
  project_setup
  project2_setup
  project3_setup

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  # Declining the batched prompt activates neither environment and prints a
  # single consolidated note, not one per environment.
  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-consent.exp" "$PROJECT_DIR" "$PROJECT3_DIR" "n" 'echo "v2:$TEST_VAR2 v3:$TEST_VAR3"'
  refute_output --partial 'v2:auto2'
  refute_output --partial 'v3:auto3'
  assert_output --partial 'Disabled auto-activation for these environments.'
  assert_equal "$(grep -c 'Disabled auto-activation' <<< "$output")" 1
}

# ---------------------------------------------------------------------------- #
# Hook auto-fires: verify the prompt hook triggers without manual invocation
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:auto-fires
@test "bash: hook auto-activates via PROMPT_COMMAND in interactive shell" {
  project_setup
  project2_setup
  # Auto-activation is opt-in; allow the target so the hook activates it
  # without prompting.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

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
  # Auto-activation is opt-in; allow every environment the run activates so the
  # hook does not prompt.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECTZ_DIR"

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

# Plain 'flox deactivate' must work even after the active environment's
# directory was deleted (e.g. a removed git worktree); the deactivation data
# is recovered from the activation state rather than the deleted directory.
# bats test_tags=hook:deactivate:deleted-dir
@test "bash: plain 'flox deactivate' tears down an environment whose directory was deleted" {
  project_setup
  project2_setup

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    cd $PROJECT2_DIR
    eval \"\$($FLOX_BIN activate -d $PROJECT2_DIR)\"
    echo \"during:\$TEST_VAR2\"
    cd $BATS_TEST_TMPDIR
    rm -rf $PROJECT2_DIR
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after:\${TEST_VAR2:-unset}\"
  "
  assert_success
  assert_output --partial "during:auto2"
  assert_output --partial "after:unset"
  refute_regex "$stderr" "Did not find an environment"
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
  # The activation is in place; its entry in the _FLOX_INVOCATION_TYPES map
  # (a shell variable, recorded by the activation) is rewritten to
  # `interactive` with tcsh's `:s` modifier so the hook requests the
  # interactive (exit) deactivation script, as it would inside a real
  # `flox activate` subshell.
  SESSION="$BATS_TEST_TMPDIR/interactive-deactivate.tcsh"
  cat > "$SESSION" <<EOF
eval "\`$FLOX_BIN activate -d $PROJECT_DIR\`"
set _FLOX_INVOCATION_TYPES="\$_FLOX_INVOCATION_TYPES:s/inplace/interactive/"
$FLOX_BIN deactivate
echo SHOULD_NOT_PRINT: the shell exits at the next prompt, before this line
EOF

  run tcsh -i < "$SESSION"
  assert_success
  refute_output --partial "Faulty alias"
  refute_output --partial "SHOULD_NOT_PRINT"
}

# ---------------------------------------------------------------------------- #
# Plain `flox deactivate` works for every layer of a nested stack
# ---------------------------------------------------------------------------- #
#
# Each in-place deactivation used to unconditionally unset the exported
# `_FLOX_PROMPT_HOOK_VERSION` marker, even when inner layers remained active and
# the prompt hook was still registered. The next `flox deactivate` then found no
# marker and wrongly aborted with "is not set up in this shell". The marker must
# never be cleared: the prompt hook stays registered even after the whole stack
# is torn down, and without the marker every later hook run with
# auto-activation work aborts with "out of sync" until the shell restarts.

# bats test_tags=hook:deactivate:nested
@test "bash: plain 'flox deactivate' works for each layer of a nested stack" {
  project2_setup
  project3_setup

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT2_DIR)\"
    eval \"\$($FLOX_BIN activate -d $PROJECT3_DIR)\"
    # Pop the inner layer; the prompt hook services the request.
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after-inner:marker=[\${_FLOX_PROMPT_HOOK_VERSION}]\"
    # The outer layer is still active and the prompt hook is still set up, so
    # this must be accepted rather than aborting.
    $FLOX_BIN deactivate || echo SECOND_DEACTIVATE_FAILED
    _flox_hook
    echo \"after-outer:[\$FLOX_PROMPT_ENVIRONMENTS]marker=[\${_FLOX_PROMPT_HOOK_VERSION}]\"
  "
  assert_success
  # The marker survives the inner deactivation (the regression).
  assert_output --partial "after-inner:marker=[1:true]"
  refute_output --partial "is not set up in this shell"
  refute_output --partial "SECOND_DEACTIVATE_FAILED"
  # The outer deactivation tore the whole stack down, and the marker survives
  # it too: the prompt hook remains registered, so a later hook run with
  # auto-activation work must not abort with "out of sync".
  assert_output --partial "after-outer:[]marker=[1:true]"
}

# bats test_tags=hook:deactivate:nested
@test "zsh: plain 'flox deactivate' works for each layer of a nested stack" {
  project2_setup
  project3_setup

  run --separate-stderr zsh -c "
    export FLOX_SHELL=\$(which zsh)
    eval \"\$($FLOX_BIN activate -d $PROJECT2_DIR)\"
    eval \"\$($FLOX_BIN activate -d $PROJECT3_DIR)\"
    $FLOX_BIN deactivate
    _flox_hook
    echo \"after-inner:marker=[\${_FLOX_PROMPT_HOOK_VERSION}]\"
    $FLOX_BIN deactivate || echo SECOND_DEACTIVATE_FAILED
    _flox_hook
    echo \"after-outer:[\$FLOX_PROMPT_ENVIRONMENTS]marker=[\${_FLOX_PROMPT_HOOK_VERSION}]\"
  "
  assert_success
  assert_output --partial "after-inner:marker=[1:true]"
  refute_output --partial "is not set up in this shell"
  refute_output --partial "SECOND_DEACTIVATE_FAILED"
  assert_output --partial "after-outer:[]marker=[1:true]"
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
    export _FLOX_PROMPT_HOOK_VERSION=99:true
    $FLOX_BIN deactivate
  "
  assert_failure
  assert_output --partial "incompatible version of Flox"
}

# bats test_tags=hook:activate:incompatible
@test "'flox activate' errors when the prompt hook version is incompatible" {
  project_setup

  run bash -c "
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    export _FLOX_PROMPT_HOOK_VERSION=99:true
    $FLOX_BIN activate -d $PROJECT_DIR
  "
  assert_failure
  assert_output --partial "incompatible version of Flox"
  # The error names the environments active in the stale shell.
  assert_output --partial "Active environments: project-${BATS_TEST_NUMBER}"
}

# bats test_tags=hook:activate:no-marker
@test "'flox activate' proceeds normally when the prompt hook marker is unset" {
  project_setup

  run bash -c "
    export FLOX_SHELL=\$(which bash)
    unset _FLOX_PROMPT_HOOK_VERSION
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    echo done
  "
  assert_success
  assert_output --partial "done"
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

# ---------------------------------------------------------------------------- #
# Mid-stack re-insertion: re-allowing a denied middle environment re-activates
# it in ancestor order, not on top of the stack.
# ---------------------------------------------------------------------------- #

# bats test_tags=hook:reinsert:bash
@test "bash: re-allowing a denied mid-stack env re-inserts it in ancestor order" {
  nested_chain_setup
  # Allow outer and inner; deny the middle so the initial stack skips it.
  "$FLOX_BIN" activate allow -d "$NEST_OUTER_DIR"
  "$FLOX_BIN" activate allow -d "$NEST_INNER_DIR"
  "$FLOX_BIN" activate deny -d "$NEST_MID_DIR"

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    cd $NEST_INNER_DIR
    eval \"\$($FLOX_BIN activate -d $NEST_OUTER_DIR)\"
    _flox_hook
    echo \"denied:[\$FLOX_PROMPT_ENVIRONMENTS] mid:\${TEST_MID:-unset}\"
    # Re-allow the middle environment; the next prompt re-inserts it.
    $FLOX_BIN activate allow -d $NEST_MID_DIR
    _flox_hook
    echo \"reallowed:[\$FLOX_PROMPT_ENVIRONMENTS] mid:\${TEST_MID:-unset}\"
    # The following prompt must be settled (a noop).
    _flox_hook
    echo \"settled:[\$FLOX_PROMPT_ENVIRONMENTS]\"
  "
  assert_success
  # Initial stack skips the denied middle: inner on top, then outer.
  assert_output --partial "denied:[inner outer] mid:unset"
  # Re-allow re-inserts mid between inner and outer (newest-first prompt order).
  assert_output --partial "reallowed:[inner mid outer] mid:mid"
  # Settled: no further reordering on the next prompt.
  assert_output --partial "settled:[inner mid outer]"
}

# bats test_tags=hook:reinsert:fish
@test "fish: re-allowing a denied mid-stack env re-inserts it in ancestor order" {
  nested_chain_setup
  "$FLOX_BIN" activate allow -d "$NEST_OUTER_DIR"
  "$FLOX_BIN" activate allow -d "$NEST_INNER_DIR"
  "$FLOX_BIN" activate deny -d "$NEST_MID_DIR"

  run --separate-stderr fish -c "
    set -gx FLOX_SHELL (which fish)
    cd $NEST_INNER_DIR
    eval \"\$($FLOX_BIN activate -d $NEST_OUTER_DIR)\"
    _flox_hook
    echo \"denied:[\$FLOX_PROMPT_ENVIRONMENTS] mid:\$TEST_MID\"
    $FLOX_BIN activate allow -d $NEST_MID_DIR
    _flox_hook
    echo \"reallowed:[\$FLOX_PROMPT_ENVIRONMENTS] mid:\$TEST_MID\"
  "
  assert_success
  assert_output --partial "denied:[inner outer]"
  assert_output --partial "reallowed:[inner mid outer] mid:mid"
}

# bats test_tags=hook:reinsert:manual-fallback
@test "bash: re-allow activates on top when a manual env is layered above the target" {
  nested_chain_setup
  # Allow only outer; deny the middle. Inner is manually activated (never
  # allowed), so it sits above the middle as a non-poppable layer.
  "$FLOX_BIN" activate allow -d "$NEST_OUTER_DIR"
  "$FLOX_BIN" activate deny -d "$NEST_MID_DIR"

  run --separate-stderr bash -c "
    export FLOX_SHELL=\$(which bash)
    cd $NEST_INNER_DIR
    eval \"\$($FLOX_BIN activate -d $NEST_OUTER_DIR)\"
    # Manually activate the inner env on top (auto-activation never tracks it).
    eval \"\$($FLOX_BIN activate -d $NEST_INNER_DIR)\"
    _flox_hook
    echo \"before:[\$FLOX_PROMPT_ENVIRONMENTS]\"
    $FLOX_BIN activate allow -d $NEST_MID_DIR
    _flox_hook
    echo \"after:[\$FLOX_PROMPT_ENVIRONMENTS] mid:\${TEST_MID:-unset}\"
  "
  assert_success
  assert_output --partial "before:[inner outer]"
  # Re-insertion is impossible across the manual inner layer, so mid activates
  # on top (out of ancestor order, but active and correct).
  assert_output --partial "after:[mid inner outer] mid:mid"
}

# ---------------------------------------------------------------------------- #
# Failure and interrupt handling: an interrupted or failed step must not leave
# a half-activated shell, and must not be retried on every subsequent prompt.
#
# The desired behavior:
#   1. Ctrl+C during a `flox activate` run by the hook activates nothing, and
#      the environments are suppressed until the directory changes.
#   2. Ctrl+C during `flox hook-env` activates nothing, and the environments
#      are suppressed until the directory changes.
#   3. An error in `flox hook-env` activates nothing, and the environments are
#      suppressed until the directory changes.
#   4. An error in a `flox activate` run by the hook suppresses that
#      environment until the directory changes, but other environments still
#      activate.
#
# That suppression clears on leaving the directory is already pinned by
# "'flox deactivate' suppresses re-activation until the directory is left"
# above, so these tests assert the failing step is (a) not applied and (b)
# recorded in _FLOX_SUPPRESSED_ENVIRONMENTS.
# ---------------------------------------------------------------------------- #

# Add hello-unfree to the manifest of the project at $1 without locking or
# building it: append straight to manifest.toml (bypassing `flox edit`) and
# drop the lockfile, so the hook's `flox activate` has to lock and build.
# hello-unfree is never in the binary cache, so the activation builds it from
# source — a long-running window in which to deliver an interrupt. That window
# only exists while the output path is absent from the store, so delete it if
# an earlier test or run built it (the marker never prints on a store hit).
add_unbuilt_package_manifest() {
  local store_path
  store_path="$(grep -oE "\"system\":\"$NIX_SYSTEM\",\"outputs\":\[\{\"name\":\"out\",\"store_path\":\"[^\"]+\"" \
    "$GENERATED_DATA/resolve/hello_unfree.yaml" | grep -oE '/nix/store/[^"]+')"
  if [ -e "$store_path" ]; then
    # Environments built from earlier runs refer to the package, so the whole
    # referrer closure has to go.
    nix-store --delete $(nix-store --query --referrers-closure "$store_path") >/dev/null 2>&1 || true
  fi
  if [ -e "$store_path" ]; then
    skip "hello-unfree is already in the store and pinned by a live root; no build to interrupt"
  fi

  cat >> "$1/.flox/env/manifest.toml" <<EOF

[install]
hello-unfree.pkg-path = "hello-unfree"
EOF
  rm -f "$1/.flox/env/manifest.lock"
  export _FLOX_USE_CATALOG_MOCK="$GENERATED_DATA/resolve/hello_unfree.yaml"
}

# A manifest for the project at $1 with an observable var and an on-activate
# hook that announces itself on stderr and then sleeps, giving the test a
# window in which to deliver Ctrl+C while `flox activate` is running.
set_slow_activate_manifest() {
  cat << EOF | "$FLOX_BIN" edit -d "$1" -f -
version = 1

[vars]
TEST_VAR2 = "auto2"

[hook]
on-activate = "echo ACTIVATE-SLEEPING >&2; sleep 8"
EOF
}

# bats test_tags=hook:interrupt:activate
@test "bash: Ctrl+C during an auto-activation activates nothing and suppresses it" {
  # Desired behavior 1, exercised with a nested pair so "no environments"
  # covers the activations queued behind the interrupted one. What already
  # works today: `flox` handles SIGINT itself (despite the hook's
  # `trap '' SIGINT`, which only silences the shell), so the in-flight outer
  # activation aborts and the next prompt's reconcile suppresses it. What
  # fails: the remaining activations emitted in the same hook run still
  # proceed, so the inner environment activates (v3:auto3) and is not
  # suppressed.
  skip "Ctrl+C aborts only the in-flight activation; the rest of the hook run's activations still proceed"

  project_setup
  project2_setup
  project3_setup
  set_slow_activate_manifest "$PROJECT2_DIR"
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before entering them.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT3_DIR" \
    'echo "v2:${TEST_VAR2:-unset} v3:${TEST_VAR3:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"' \
    "ACTIVATE-SLEEPING"
  # Neither the interrupted outer activation nor the queued inner one applies ...
  assert_output --partial "v2:unset"
  assert_output --partial "v3:unset"
  # ... and both are suppressed so the next prompt does not retry them. Match
  # the JSON-quoted elements of the `sup:` list; the bare paths also appear in
  # the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
  assert_output --partial "\"$(realpath "$PROJECT3_DIR")\""
}

# bats test_tags=hook:interrupt:activate
@test "zsh: Ctrl+C during an auto-activation activates nothing and suppresses it" {
  # Fails the same way as the bash test above: the in-flight outer activation
  # aborts and is later suppressed, but the inner activation queued in the
  # same hook run still proceeds.
  skip "Ctrl+C aborts only the in-flight activation; the rest of the hook run's activations still proceed"

  project_setup
  project2_setup
  project3_setup
  set_slow_activate_manifest "$PROJECT2_DIR"
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before entering them.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"

  # Set up a .zshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.zshrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF

  FLOX_SHELL="zsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT3_DIR" \
    'echo "v2:${TEST_VAR2:-unset} v3:${TEST_VAR3:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"' \
    "ACTIVATE-SLEEPING"
  # Neither the interrupted outer activation nor the queued inner one applies ...
  assert_output --partial "v2:unset"
  assert_output --partial "v3:unset"
  # ... and both are suppressed so the next prompt does not retry them. Match
  # the JSON-quoted elements of the `sup:` list; the bare paths also appear in
  # the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
  assert_output --partial "\"$(realpath "$PROJECT3_DIR")\""
}

# bats test_tags=hook:interrupt:activate
@test "fish: Ctrl+C during an auto-activation activates nothing and suppresses it" {
  # The fish hook has no SIGINT protection around its eval, so Ctrl+C aborts
  # the whole hook run before any tracking is recorded. The next prompt then
  # retries from scratch: the slow on-activate runs again — blocking the
  # prompt until it finishes — and both environments end up activated with
  # nothing suppressed.
  skip "Ctrl+C aborts the hook run before anything is recorded, so the next prompt retries and activates everything"

  project_setup
  project2_setup
  project3_setup
  set_slow_activate_manifest "$PROJECT2_DIR"
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before entering them.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"

  # Set up a config.fish so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  mkdir -p "$HOME/.config/fish"
  cat >"$HOME/.config/fish/config.fish" <<EOF
function fish_prompt
  echo -n "$KNOWN_PROMPT"
end
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not fish syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="fish" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT3_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} v3:${TEST_VAR3:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\''' \
    "ACTIVATE-SLEEPING"
  # Neither the interrupted outer activation nor the queued inner one applies ...
  assert_output --partial "v2:unset"
  assert_output --partial "v3:unset"
  # ... and both are suppressed so the next prompt does not retry them. Match
  # the JSON-quoted elements of the `sup:` list; the bare paths also appear in
  # the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
  assert_output --partial "\"$(realpath "$PROJECT3_DIR")\""
}

# bats test_tags=hook:interrupt:activate
@test "tcsh: Ctrl+C during an auto-activation activates nothing and suppresses it" {
  # tcsh is the one shell where this works today: the hook-env output reaches
  # the shell as a single eval'd line whose nested backquote substitutions run
  # the activates, so the interrupt kills the substitutions (neither
  # environment produces an activate script) while the line's plain setenv
  # statements still execute and record both as tracked. The next prompt's
  # reconcile finds them tracked but not active and suppresses both.
  project_setup
  project2_setup
  project3_setup
  set_slow_activate_manifest "$PROJECT2_DIR"
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before entering them.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"

  # Set up a .tcshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.tcshrc" <<EOF
set prompt = "$KNOWN_PROMPT"
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not tcsh syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="tcsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT3_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} v3:${TEST_VAR3:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\''' \
    "ACTIVATE-SLEEPING"
  # Neither the interrupted outer activation nor the queued inner one applies ...
  assert_output --partial "v2:unset"
  assert_output --partial "v3:unset"
  # ... and both are suppressed so the next prompt does not retry them. Match
  # the JSON-quoted elements of the `sup:` list; the bare paths also appear in
  # the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
  assert_output --partial "\"$(realpath "$PROJECT3_DIR")\""
}

# bats test_tags=hook:interrupt:build
@test "bash: Ctrl+C during a package build in an auto-activation activates nothing and suppresses it" {
  # Desired behavior 1, with the interrupt landing in the package-build phase
  # of the hook-run `flox activate` instead of its on-activate hook.
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  add_unbuilt_package_manifest "$PROJECT2_DIR"

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"' \
    "Building 'hello-unfree' from source"
  # The interrupted activation must not apply ...
  assert_output --partial "v2:unset"
  # ... and must be suppressed so the next prompt does not restart the build.
  # Match the JSON-quoted element of the `sup:` list; the bare path also
  # appears in the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:build
@test "zsh: Ctrl+C during a package build in an auto-activation activates nothing and suppresses it" {
  # Desired behavior 1, with the interrupt landing in the package-build phase
  # of the hook-run `flox activate` instead of its on-activate hook.
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  add_unbuilt_package_manifest "$PROJECT2_DIR"

  # Set up a .zshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.zshrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF

  FLOX_SHELL="zsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"' \
    "Building 'hello-unfree' from source"
  # The interrupted activation must not apply ...
  assert_output --partial "v2:unset"
  # ... and must be suppressed so the next prompt does not restart the build.
  # Match the JSON-quoted element of the `sup:` list; the bare path also
  # appears in the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:build
@test "fish: Ctrl+C during a package build in an auto-activation activates nothing and suppresses it" {
  # Desired behavior 1, with the interrupt landing in the package-build phase
  # of the hook-run `flox activate` instead of its on-activate hook. Fails
  # like the fish on-activate test above: the interrupt aborts the hook run
  # before any tracking is recorded, so the next prompt restarts the build
  # from scratch and the environment activates with nothing suppressed.
  skip "Ctrl+C aborts the hook run before anything is recorded, so the next prompt rebuilds and activates the environment"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  add_unbuilt_package_manifest "$PROJECT2_DIR"

  # Set up a config.fish so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  mkdir -p "$HOME/.config/fish"
  cat >"$HOME/.config/fish/config.fish" <<EOF
function fish_prompt
  echo -n "$KNOWN_PROMPT"
end
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not fish syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="fish" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\''' \
    "Building 'hello-unfree' from source"
  # The interrupted activation must not apply ...
  assert_output --partial "v2:unset"
  # ... and must be suppressed so the next prompt does not restart the build.
  # Match the JSON-quoted element of the `sup:` list; the bare path also
  # appears in the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:build
@test "tcsh: Ctrl+C during a package build in an auto-activation activates nothing and suppresses it" {
  # Desired behavior 1, with the interrupt landing in the package-build phase
  # of the hook-run `flox activate` instead of its on-activate hook.
  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  add_unbuilt_package_manifest "$PROJECT2_DIR"

  # Set up a .tcshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.tcshrc" <<EOF
set prompt = "$KNOWN_PROMPT"
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not tcsh syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="tcsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-activate.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\''' \
    "Building 'hello-unfree' from source"
  # The interrupted activation must not apply ...
  assert_output --partial "v2:unset"
  # ... and must be suppressed so the next prompt does not restart the build.
  # Match the JSON-quoted element of the `sup:` list; the bare path also
  # appears in the echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:hook-env
@test "bash: SIGINT during hook-env activates nothing and suppresses the environment" {
  # Desired behavior 2. The signal is delivered with `pkill -INT` while
  # hook-env blocks at the consent prompt (see hook-interrupt-hook-env.exp for
  # why a keyboard Ctrl+C would not interrupt hook-env there). Currently the
  # interrupted hook-env prints "user interrupted process" but never exits —
  # the consent prompt's blocking terminal read keeps the process alive — so
  # the shell stays blocked in the hook's command substitution and never gets
  # a prompt back.
  skip "a SIGINT'd hook-env hangs instead of exiting, leaving the shell blocked at the prompt"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # No allow/deny recorded, so the default 'prompt' mode asks for consent.

  # Set up a .bashrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.bashrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF
  cat >"$HOME/.inputrc" <<EOF
set enable-bracketed-paste off
EOF

  FLOX_SHELL="bash" run -0 expect "$TESTS_DIR/activate/hook-interrupt-hook-env.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'
  # The interrupted consent must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not ask again. Match the
  # JSON-quoted element of the `sup:` list; the bare path also appears in the
  # echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:hook-env
@test "zsh: SIGINT during hook-env activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: the SIGINT'd hook-env never
  # exits, so the shell stays blocked in the hook.
  skip "a SIGINT'd hook-env hangs instead of exiting, leaving the shell blocked at the prompt"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # No allow/deny recorded, so the default 'prompt' mode asks for consent.

  # Set up a .zshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.zshrc" <<EOF
export PS1="$KNOWN_PROMPT"
EOF

  FLOX_SHELL="zsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-hook-env.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'
  # The interrupted consent must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not ask again. Match the
  # JSON-quoted element of the `sup:` list; the bare path also appears in the
  # echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:hook-env
@test "fish: SIGINT during hook-env activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: the SIGINT'd hook-env never
  # exits, so the shell stays blocked in the hook.
  skip "a SIGINT'd hook-env hangs instead of exiting, leaving the shell blocked at the prompt"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # No allow/deny recorded, so the default 'prompt' mode asks for consent.

  # Set up a config.fish so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  mkdir -p "$HOME/.config/fish"
  cat >"$HOME/.config/fish/config.fish" <<EOF
function fish_prompt
  echo -n "$KNOWN_PROMPT"
end
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not fish syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="fish" run -0 expect "$TESTS_DIR/activate/hook-interrupt-hook-env.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\'''
  # The interrupted consent must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not ask again. Match the
  # JSON-quoted element of the `sup:` list; the bare path also appears in the
  # echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:interrupt:hook-env
@test "tcsh: SIGINT during hook-env activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: the SIGINT'd hook-env never
  # exits, so the shell stays blocked in the hook.
  skip "a SIGINT'd hook-env hangs instead of exiting, leaving the shell blocked at the prompt"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # No allow/deny recorded, so the default 'prompt' mode asks for consent.

  # Set up a .tcshrc so the interactive shell has a known prompt
  export KNOWN_PROMPT="hooktest> "
  cat >"$HOME/.tcshrc" <<EOF
set prompt = "$KNOWN_PROMPT"
EOF

  # The observation runs through `sh` because the \${VAR:-unset} fallback is
  # not tcsh syntax; the vars are exported so a child sh sees them.
  FLOX_SHELL="tcsh" run -0 expect "$TESTS_DIR/activate/hook-interrupt-hook-env.exp" \
    "$PROJECT_DIR" "$PROJECT2_DIR" \
    'sh -c '\''echo "v2:${TEST_VAR2:-unset} sup:${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}"'\'''
  # The interrupted consent must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not ask again. Match the
  # JSON-quoted element of the `sup:` list; the bare path also appears in the
  # echoed `cd` command in the transcript.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:error:hook-env
@test "bash: a hook-env error activates nothing and suppresses the environment" {
  # Desired behavior 3. Currently fails the suppression half: hook-env bails
  # before emitting anything (nothing activates), but no suppression is
  # recorded, so every subsequent prompt re-runs hook-env and either fails
  # again or activates the environment once the error clears.
  skip "a hook-env error activates nothing but is retried on every prompt instead of suppressing"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  # Make hook-env fail persistently: its first step is to read this shell's
  # prompt-hook action file, so turning that path into a directory makes the
  # read error (and, unlike a malformed file, it cannot be consumed). The
  # runtime dir is XDG_RUNTIME_DIR/flox when the xdg crate accepts
  # XDG_RUNTIME_DIR (it must be user-owned with mode 0700) and
  # FLOX_CACHE_DIR/run otherwise; plant the directory under both.
  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    mkdir -p \"\$XDG_RUNTIME_DIR/flox/prompt-hook-actions/\$\$.json\"
    mkdir -p \"\$FLOX_CACHE_DIR/run/prompt-hook-actions/\$\$.json\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"v2:\${TEST_VAR2:-unset}\"
    echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  # The failed hook run must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not retry.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:error:hook-env
@test "zsh: a hook-env error activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: nothing activates, but no
  # suppression is recorded, so every prompt retries the failing hook-env.
  skip "a hook-env error activates nothing but is retried on every prompt instead of suppressing"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  # Make hook-env fail persistently: its first step is to read this shell's
  # prompt-hook action file, so turning that path into a directory makes the
  # read error (and, unlike a malformed file, it cannot be consumed). The
  # runtime dir is XDG_RUNTIME_DIR/flox when the xdg crate accepts
  # XDG_RUNTIME_DIR (it must be user-owned with mode 0700) and
  # FLOX_CACHE_DIR/run otherwise; plant the directory under both.
  run --separate-stderr zsh -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which zsh)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    mkdir -p \"\$XDG_RUNTIME_DIR/flox/prompt-hook-actions/\$\$.json\"
    mkdir -p \"\$FLOX_CACHE_DIR/run/prompt-hook-actions/\$\$.json\"
    cd $PROJECT2_DIR
    _flox_hook
    echo \"v2:\${TEST_VAR2:-unset}\"
    echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  # The failed hook run must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not retry.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:error:hook-env
@test "fish: a hook-env error activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: nothing activates, but no
  # suppression is recorded, so every prompt retries the failing hook-env.
  skip "a hook-env error activates nothing but is retried on every prompt instead of suppressing"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  # Same persistent hook-env failure as the zsh test above; the observation
  # runs through `sh` because the \${VAR:-unset} fallback is not fish syntax.
  run --separate-stderr fish -c "
    set -gx FLOX_FEATURES_AUTO_ACTIVATE true
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    mkdir -p \$XDG_RUNTIME_DIR/flox/prompt-hook-actions/\$fish_pid.json
    mkdir -p \$FLOX_CACHE_DIR/run/prompt-hook-actions/\$fish_pid.json
    cd $PROJECT2_DIR
    _flox_hook
    sh -c 'echo \"v2:\${TEST_VAR2:-unset}\"; echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"'
  "
  assert_success
  # The failed hook run must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not retry.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:error:hook-env
@test "tcsh: a hook-env error activates nothing and suppresses the environment" {
  # Fails the same way as the bash test above: nothing activates, but no
  # suppression is recorded, so every prompt retries the failing hook-env.
  skip "a hook-env error activates nothing but is retried on every prompt instead of suppressing"

  project_setup
  project2_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow the target before entering it.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"

  # Same persistent hook-env failure as the zsh test above; the observation
  # runs through `sh` because the \${VAR:-unset} fallback is not tcsh syntax.
  run --separate-stderr tcsh -c "
    setenv FLOX_FEATURES_AUTO_ACTIVATE true
    eval \"\`$FLOX_BIN activate -d $PROJECT_DIR\`\"
    mkdir -p \"\$XDG_RUNTIME_DIR/flox/prompt-hook-actions/\$\$.json\"
    mkdir -p \"\$FLOX_CACHE_DIR/run/prompt-hook-actions/\$\$.json\"
    cd $PROJECT2_DIR
    precmd
    sh -c 'echo \"v2:\${TEST_VAR2:-unset}\"; echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"'
  "
  assert_success
  # The failed hook run must not activate the environment ...
  assert_output --partial "v2:unset"
  # ... and must suppress it so the next prompt does not retry.
  assert_output --partial "\"$(realpath "$PROJECT2_DIR")\""
}

# bats test_tags=hook:error:activate
@test "bash: a failing auto-activation is suppressed while other environments still activate" {
  # Desired behavior 4: entering a nested pair where the outer environment is
  # broken must still activate the inner one, and the next prompt must move
  # the broken environment from tracked to suppressed rather than retrying it.
  project_setup
  project2_setup
  project3_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before breaking the outer.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"
  # Break the outer environment so its `flox activate` exits non-zero: an
  # unparseable manifest with no lockfile fails at lock time.
  echo 'not valid toml [' > "$PROJECT2_DIR/.flox/env/manifest.toml"
  rm -f "$PROJECT2_DIR/.flox/env/manifest.lock"

  run --separate-stderr bash -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which bash)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT3_DIR
    _flox_hook
    echo \"first: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"
    _flox_hook
    echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
    echo \"second: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"left:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  # The broken outer environment does not activate; the inner one does.
  assert_output --partial "first: v2:unset v3:auto3"
  # The next prompt suppresses the broken environment instead of retrying it,
  # and leaves the inner activation in place.
  assert_output --partial "sup:[\"$(realpath "$PROJECT2_DIR")\"]"
  assert_output --partial "second: v2:unset v3:auto3"
  # Leaving the directory clears the suppression.
  assert_output --partial "left:unset"
}

# bats test_tags=hook:error:activate
@test "zsh: a failing auto-activation is suppressed while other environments still activate" {
  # Desired behavior 4, as above.
  project_setup
  project2_setup
  project3_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before breaking the outer.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"
  # Break the outer environment so its `flox activate` exits non-zero: an
  # unparseable manifest with no lockfile fails at lock time.
  echo 'not valid toml [' > "$PROJECT2_DIR/.flox/env/manifest.toml"
  rm -f "$PROJECT2_DIR/.flox/env/manifest.lock"

  run --separate-stderr zsh -c "
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export FLOX_SHELL=\$(which zsh)
    eval \"\$($FLOX_BIN activate -d $PROJECT_DIR)\"
    cd $PROJECT3_DIR
    _flox_hook
    echo \"first: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"
    _flox_hook
    echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
    echo \"second: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"
    cd $BATS_TEST_TMPDIR
    _flox_hook
    echo \"left:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"
  "
  assert_success
  # The broken outer environment does not activate; the inner one does.
  assert_output --partial "first: v2:unset v3:auto3"
  # The next prompt suppresses the broken environment instead of retrying it,
  # and leaves the inner activation in place.
  assert_output --partial "sup:[\"$(realpath "$PROJECT2_DIR")\"]"
  assert_output --partial "second: v2:unset v3:auto3"
  # Leaving the directory clears the suppression.
  assert_output --partial "left:unset"
}

# bats test_tags=hook:error:activate
@test "fish: a failing auto-activation is suppressed while other environments still activate" {
  # Desired behavior 4, as above. The observations run through `sh` because
  # the \${VAR:-unset} fallback is not fish syntax.
  project_setup
  project2_setup
  project3_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before breaking the outer.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"
  # Break the outer environment so its `flox activate` exits non-zero: an
  # unparseable manifest with no lockfile fails at lock time.
  echo 'not valid toml [' > "$PROJECT2_DIR/.flox/env/manifest.toml"
  rm -f "$PROJECT2_DIR/.flox/env/manifest.lock"

  run --separate-stderr fish -c "
    set -gx FLOX_FEATURES_AUTO_ACTIVATE true
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    cd $PROJECT3_DIR
    _flox_hook
    sh -c 'echo \"first: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"'
    _flox_hook
    sh -c 'echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"; echo \"second: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"'
    cd $BATS_TEST_TMPDIR
    _flox_hook
    sh -c 'echo \"left:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"'
  "
  assert_success
  # The broken outer environment does not activate; the inner one does.
  assert_output --partial "first: v2:unset v3:auto3"
  # The next prompt suppresses the broken environment instead of retrying it,
  # and leaves the inner activation in place.
  assert_output --partial "sup:[\"$(realpath "$PROJECT2_DIR")\"]"
  assert_output --partial "second: v2:unset v3:auto3"
  # Leaving the directory clears the suppression.
  assert_output --partial "left:unset"
}

# bats test_tags=hook:error:activate
@test "tcsh: a failing auto-activation is suppressed while other environments still activate" {
  # Desired behavior 4, as above. The observations run through `sh` because
  # the \${VAR:-unset} fallback is not tcsh syntax. tcsh also fires cwdcmd on
  # each cd, so the manual precmd calls are not necessarily the first hook run
  # after a directory change; the assertions hold either way.
  project_setup
  project2_setup
  project3_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  # Auto-activation is opt-in; allow both layers before breaking the outer.
  "$FLOX_BIN" activate allow -d "$PROJECT2_DIR"
  "$FLOX_BIN" activate allow -d "$PROJECT3_DIR"
  # Break the outer environment so its `flox activate` exits non-zero: an
  # unparseable manifest with no lockfile fails at lock time.
  echo 'not valid toml [' > "$PROJECT2_DIR/.flox/env/manifest.toml"
  rm -f "$PROJECT2_DIR/.flox/env/manifest.lock"

  run --separate-stderr tcsh -c "
    setenv FLOX_FEATURES_AUTO_ACTIVATE true
    eval \"\`$FLOX_BIN activate -d $PROJECT_DIR\`\"
    cd $PROJECT3_DIR
    precmd
    sh -c 'echo \"first: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"'
    precmd
    sh -c 'echo \"sup:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"; echo \"second: v2:\${TEST_VAR2:-unset} v3:\${TEST_VAR3:-unset}\"'
    cd $BATS_TEST_TMPDIR
    precmd
    sh -c 'echo \"left:\${_FLOX_SUPPRESSED_ENVIRONMENTS:-unset}\"'
  "
  assert_success
  # The broken outer environment does not activate; the inner one does.
  assert_output --partial "first: v2:unset v3:auto3"
  # The next prompt suppresses the broken environment instead of retrying it,
  # and leaves the inner activation in place.
  assert_output --partial "sup:[\"$(realpath "$PROJECT2_DIR")\"]"
  assert_output --partial "second: v2:unset v3:auto3"
  # Leaving the directory clears the suppression.
  assert_output --partial "left:unset"
}
