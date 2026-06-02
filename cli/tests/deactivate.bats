#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox deactivate' subcommand.
# We are especially interested in ensuring that the deactivation script properly
# restores environment variables and cleans up after activation.
#
#
# ---------------------------------------------------------------------------- #

load test_support.bash

# bats file_tags=deactivate

# ---------------------------------------------------------------------------- #

setup_file() {
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  common_file_setup
}

# ---------------------------------------------------------------------------- #

project_setup_common() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project-${BATS_TEST_NUMBER?}"
  export PROJECT_NAME="${PROJECT_DIR##*/}"

  rm -rf "$PROJECT_DIR"
  mkdir -p "$PROJECT_DIR"
  pushd "$PROJECT_DIR" >/dev/null || return

}

project_setup() {
  project_setup_common
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
    wait_for_activations "$PROJECT_DIR" || return 1
    project_teardown
  fi
  common_test_teardown
}

# ---------------------------------------------------------------------------- #

# bats test_tags=deactivate
@test "deactivate restores environment variables (bash)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - Set TEST_VAR=original before activation
  # - The [vars] section changes TEST_VAR=modified
  # - After deactivation, TEST_VAR should be restored to "original"

  FLOX_SHELL="bash" run --separate-stderr bash -c '
    export TEST_VAR=original
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (fish)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - Set TEST_VAR=original before activation
  # - The [vars] section changes TEST_VAR=modified
  # - After deactivation, TEST_VAR should be restored to "original"

  SHELL="$(which fish)" run --separate-stderr fish -c '
    set -gx TEST_VAR original
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (tcsh)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - Set TEST_VAR=original before activation
  # - The [vars] section changes TEST_VAR=modified
  # - After deactivation, TEST_VAR should be restored to "original"

  SHELL="$(which tcsh)" run --separate-stderr tcsh -c '
    setenv TEST_VAR original
    eval "`$FLOX_BIN activate --print-script`"
    echo "during:$TEST_VAR"
    eval "`$FLOX_BIN deactivate --print-script $_FLOX_INVOCATION_TYPE`"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (zsh)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - Set TEST_VAR=original before activation
  # - The [vars] section changes TEST_VAR=modified
  # - After deactivation, TEST_VAR should be restored to "original"

  FLOX_SHELL="zsh" run --separate-stderr zsh -c '
    export TEST_VAR=original
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate unsets added variables (bash)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - TEST_NEW_VAR does not exist before activation
  # - The on-activate hook exports TEST_NEW_VAR=newly_added
  # - After deactivation, TEST_NEW_VAR should be unset

  FLOX_SHELL="bash" run --separate-stderr bash -c '
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_NEW_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    if [ -z "${TEST_NEW_VAR+x}" ]; then
      echo "after:unset"
    fi
  '
  assert_success
  assert_line "during:newly_added"
  assert_line "after:unset"
}

# bats test_tags=deactivate
@test "deactivate unsets added variables (fish)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - TEST_NEW_VAR does not exist before activation
  # - The on-activate hook exports TEST_NEW_VAR=newly_added
  # - After deactivation, TEST_NEW_VAR should be unset

  SHELL="$(which fish)" run --separate-stderr fish -c '
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_NEW_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    if not set -q TEST_NEW_VAR
      echo "after:unset"
    end
  '
  assert_success
  assert_line "during:newly_added"
  assert_line "after:unset"
}

# bats test_tags=deactivate
@test "deactivate unsets added variables (tcsh)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - TEST_NEW_VAR does not exist before activation
  # - The on-activate hook exports TEST_NEW_VAR=newly_added
  # - After deactivation, TEST_NEW_VAR should be unset

  SHELL="$(which tcsh)" run --separate-stderr tcsh -c '
    eval "`$FLOX_BIN activate --print-script`"
    echo "during:$TEST_NEW_VAR"
    eval "`$FLOX_BIN deactivate --print-script $_FLOX_INVOCATION_TYPE`"
    if ( ! $?TEST_NEW_VAR ) then
      echo "after:unset"
    endif
  '
  assert_success
  assert_line "during:newly_added"
  assert_line "after:unset"
}

# bats test_tags=deactivate
@test "deactivate unsets added variables (zsh)" {
  project_setup
  MANIFEST_CONTENTS="$(cat << "EOF"
version = 1

[vars]
TEST_VAR = "modified"

[hook]
on-activate = """
  export TEST_NEW_VAR="newly_added"
"""
EOF
  )"

  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -f -

  # What this is testing:
  # - TEST_NEW_VAR does not exist before activation
  # - The on-activate hook exports TEST_NEW_VAR=newly_added
  # - After deactivation, TEST_NEW_VAR should be unset

  FLOX_SHELL="zsh" run --separate-stderr zsh -c '
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$TEST_NEW_VAR"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    if [ -z "${TEST_NEW_VAR+x}" ]; then
      echo "after:unset"
    fi
  '
  assert_success
  assert_line "during:newly_added"
  assert_line "after:unset"
}

# bats test_tags=activate,deactivate
@test "deactivate is no-op without activation" {
  skip "deactivate --print-script not yet implemented"
  project_setup

  # What this is testing:
  # - When _FLOX_HOOK_DIFF doesn't exist (no prior activation)
  # - deactivate should output nothing and succeed
  # - Environment variables should remain unchanged

  run bash -c '
    export TEST_VAR=unchanged
    eval "$($FLOX_BIN deactivate --print-script)"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "after:unchanged"
}

# ---------------------------------------------------------------------------- #
# Prompt tests
# ---------------------------------------------------------------------------- #

# Extract content from the first match for <tag>...content...</tag>
extract_tagged_content() {
  local output="${1?}"
  shift
  local tag="${1?}"
  shift
  local match
  match=$(grep -o -m1 "<${tag}>.*</${tag}>" <<< "$output")
  match=${match#"<${tag}>"}
  match=${match%"</${tag}>"}
  echo -n "$match"
}

# Each test's inner shell wraps the prompt observed at each phase of the
# round-trip in tags:
#
#     <before>PROMPT</before>
#     <active>PROMPT</active>
#     <after>PROMPT</after>
assert_prompt_round_trip() {
  local output="${1?}"
  shift

  local before active after
  before=$(extract_tagged_content "$output" before)
  active=$(extract_tagged_content "$output" active)
  after=$(extract_tagged_content "$output" after)

  [ -n "$before" ]
  [ -n "$active" ]
  [ -n "$after" ]

  assert_not_equal "$before" "$active"
  assert_equal "$before" "$after"
}


# bats test_tags=deactivate,deactivate:prompt,deactivate:prompt:bash
@test "bash: deactivate --print-script restores prompt" {
  project_setup
  run unbuffer bash --norc --noprofile -c '
    export PS1="knownPrompt> "
    echo "<before>$PS1</before>"
    eval "$("$FLOX_BIN" activate -d "$PROJECT_DIR")"
    echo "<active>$PS1</active>"
    eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "<after>$PS1</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# bats test_tags=deactivate,deactivate:prompt,deactivate:prompt:zsh
@test "zsh: deactivate --print-script restores prompt" {
  project_setup
  run unbuffer zsh -f -i -c '
    export PS1="knownPrompt> "
    echo "<before>$PS1</before>"
    eval "$("$FLOX_BIN" activate -d "$PROJECT_DIR")"
    echo "<active>$PS1</active>"
    eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPE")"
    echo "<after>$PS1</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# bats test_tags=deactivate,deactivate:prompt,deactivate:prompt:fish
@test "fish: deactivate --print-script restores prompt" {
  project_setup
  run unbuffer fish -c '
    function fish_prompt; echo -n "knownPrompt> "; end
    echo "<before>"(fish_prompt)"</before>"
    eval ($FLOX_BIN activate -d $PROJECT_DIR)
    echo "<active>"(fish_prompt)"</active>"
    eval ($FLOX_BIN deactivate --print-script $_FLOX_INVOCATION_TYPE)
    echo "<after>"(fish_prompt)"</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# bats test_tags=deactivate,deactivate:prompt,deactivate:prompt:tcsh
@test "tcsh: deactivate --print-script restores prompt" {
  project_setup
  run unbuffer tcsh -c '
    set prompt = "knownPrompt> "
    echo "<before>$prompt</before>"
    eval "`$FLOX_BIN activate -d $PROJECT_DIR`"
    echo "<active>$prompt</active>"
    eval "`$FLOX_BIN deactivate --print-script $_FLOX_INVOCATION_TYPE`"
    echo "<after>$prompt</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# ---------------------------------------------------------------------------- #
# end prompt tests
# ---------------------------------------------------------------------------- #

# FLOX_SHELL is user-controlled: flox reads it to pick the shell but never sets
# it on the activated shell. Deactivate must therefore leave the user's
# pre-activation state exactly as it was.

# bats test_tags=deactivate
@test "deactivate preserves a user-set FLOX_SHELL (zsh)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  # The user explicitly set FLOX_SHELL=zsh before activating; after
  # deactivation it must still be "zsh".
  run --separate-stderr zsh -c '
    export FLOX_SHELL=zsh
    eval "$($FLOX_BIN activate --print-script)"
    echo "during:$FLOX_SHELL"
    eval "$($FLOX_BIN deactivate --print-script)"
    echo "after:$FLOX_SHELL"
  '
  assert_success
  assert_line "during:zsh"
  assert_line "after:zsh"
}

# bats test_tags=deactivate
@test "deactivate does not leak FLOX_SHELL when user did not set it (zsh)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  # The user never set FLOX_SHELL (shell selection is driven by SHELL); after
  # deactivation FLOX_SHELL must remain unset rather than leak in.
  SHELL="$(command -v zsh)" run --separate-stderr zsh -c '
    unset FLOX_SHELL
    eval "$($FLOX_BIN activate --print-script)"
    if [ -z "${FLOX_SHELL+x}" ]; then echo "during:unset"; else echo "during:$FLOX_SHELL"; fi
    eval "$($FLOX_BIN deactivate --print-script)"
    if [ -z "${FLOX_SHELL+x}" ]; then echo "after:unset"; else echo "after:$FLOX_SHELL"; fi
  '
  assert_success
  assert_line "during:unset"
  assert_line "after:unset"
}

# ---------------------------------------------------------------------------- #

# Full-environment diff tests. These capture `env` before activation and
# after deactivation, then assert the set of vars whose value changed
# (or that were added/removed) matches the inline expected list. Treat
# the expected list as a TODO -- when a fix lands, shrink it.
#
# Absolute tool paths are captured up front because activation can leave
# PATH broken; we want the test scaffold to keep working so we can see
# exactly which vars leaked.
#
# The inner shell is started with `env -u` for the flox-internal vars below
# to force a COLD START. Without this, when the test runner is itself inside
# a flox activation (the common local case), those vars are already present
# in `before`; the activation diff then classifies them as `modified` and
# RESTORES them on deactivate, so a genuine leak shows as "unchanged" and the
# test silently passes. Unsetting them makes the diff treat them as `added`,
# so a failure to unset them on deactivate surfaces as a real leak.
FLOX_COLD_START_UNSET=(
  -u _activate_d
  -u _flox_activations
  -u _flox_activate_tracer
  -u _FLOX_ACTIVE_ENVIRONMENTS
)

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (bash)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN

  FLOX_SHELL="bash" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" bash -c '
    "$ENV_BIN" | "$SORT_BIN" > before
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script)"
    "$ENV_BIN" | "$SORT_BIN" > after
    "$COMM_BIN" -3 before after | "$TR_BIN" -d "\t" | "$CUT_BIN" -d= -f1 | "$SORT_BIN" -u
  '
  assert_success
  assert_output - <<EOF
_activate_d
_flox_activate_tracer
EOF
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (fish)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN

  SHELL="$(which fish)" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" fish -c '
    "$ENV_BIN" | "$SORT_BIN" > before
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script)"
    "$ENV_BIN" | "$SORT_BIN" > after
    "$COMM_BIN" -3 before after | "$TR_BIN" -d "\t" | "$CUT_BIN" -d= -f1 | "$SORT_BIN" -u
  '
  assert_success
  assert_output - <<EOF
_activate_d
_flox_activate_tracer
EOF
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (tcsh)" {
  skip "tcsh fails due to FLOX_PROMPT_ENVIRONMENTS undefined variable issue"
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN

  SHELL="$(which tcsh)" run --separate-stderr tcsh -c '
    "$ENV_BIN" | "$SORT_BIN" > before
    eval "`$FLOX_BIN activate --print-script`"
    eval "`$FLOX_BIN deactivate --print-script`"
    "$ENV_BIN" | "$SORT_BIN" > after
    "$COMM_BIN" -3 before after | "$TR_BIN" -d "\t" | "$CUT_BIN" -d= -f1 | "$SORT_BIN" -u
  '
  assert_success
  assert_output ""
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (zsh)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN

  FLOX_SHELL="zsh" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" zsh -c '
    "$ENV_BIN" | "$SORT_BIN" > before
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script)"
    "$ENV_BIN" | "$SORT_BIN" > after
    "$COMM_BIN" -3 before after | "$TR_BIN" -d "\t" | "$CUT_BIN" -d= -f1 | "$SORT_BIN" -u
  '
  assert_success
  # zsh emits `_activate_d` via a non-export helper, so it does not leak here
  # (unlike bash/fish). Only `_flox_activate_tracer` survives deactivate.
  assert_output - <<EOF
_flox_activate_tracer
EOF
}

# Subshell-mode counterparts: `flox activate -c "..."` runs the body in
# the activated subshell. The body deactivates in-place inside that
# subshell and dumps env, which is captured as `after`; `before` is the
# PARENT shell's pre-activation env.
#
# IMPORTANT — read the expected blocks below with care. These compare TWO
# DIFFERENT shells (parent's pre-activation env vs. the flox-spawned child
# subshell's env), NOT the same shell before and after deactivate. As a
# result the diff is NOT a clean leak list: it also surfaces
#   - vars flox legitimately sets for the activated session (e.g.
#     SSL_CERT_FILE) — correct while activated, not a leak,
#   - vars from the flox process itself that bleed into the capture (e.g.
#     FLOX_VERSION, FLOX_SHELL, _FLOX_*_VERBOSITY) — never reach the user's
#     real shell,
#   - benign parent/child differences (SHLVL, etc.).
# None of FLOX_VERSION / FLOX_SHELL / SSL_CERT_FILE actually leak into the
# user's shell: after a real in-place deactivate, and in the parent shell
# after `flox activate -c` returns, they are all clean/unchanged. The
# in-place env-diff tests above are the trustworthy same-shell leak
# detectors; use THOSE when shrinking leaks.
#
# These subshell tests are therefore kept mainly to exercise the
# `flox activate -c` entry path and pin its current behavior. They add
# little leak-detection value over the in-place tests and are a candidate
# for deletion — if they start churning on unrelated parent/child env
# differences, prefer removing them over expanding the expected blocks.
#
# The outer shell is always bash; FLOX_SHELL/SHELL controls the
# activated shell. Running `flox activate -c` from inside a zsh `-c`
# wrapper hits an unrelated test-env PATH issue, and `env -0` requires
# coreutils so we use bash regardless of the shell under test.

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (bash)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  BODY='eval "$($FLOX_BIN deactivate --print-script)"; $ENV_BIN -0 | $SORT_BIN -z'
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN BODY

  FLOX_SHELL="bash" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" bash -c '
    "$ENV_BIN" -0 | "$SORT_BIN" -z > before
    "$FLOX_BIN" activate -c "$BODY" > after
    "$COMM_BIN" -z -3 before after | "$CUT_BIN" -z -d= -f1 | "$TR_BIN" -d "\t" | "$SORT_BIN" -uz | "$TR_BIN" "\0" "\n"
  '
  assert_success
  assert_output - <<EOF
FLOX_SHELL
FLOX_VERSION
SHLVL
SSL_CERT_FILE
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (fish)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  BODY='eval "$($FLOX_BIN deactivate --print-script)"; $ENV_BIN -0 | $SORT_BIN -z'
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN BODY

  SHELL="$(which fish)" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" bash -c '
    "$ENV_BIN" -0 | "$SORT_BIN" -z > before
    "$FLOX_BIN" activate -c "$BODY" > after
    "$COMM_BIN" -z -3 before after | "$CUT_BIN" -z -d= -f1 | "$TR_BIN" -d "\t" | "$SORT_BIN" -uz | "$TR_BIN" "\0" "\n"
  '
  assert_success
  assert_output - <<EOF
FLOX_VERSION
SSL_CERT_FILE
_
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (tcsh)" {
  skip "tcsh fails due to FLOX_PROMPT_ENVIRONMENTS undefined variable issue"
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  BODY='eval "`$FLOX_BIN deactivate --print-script`"; $ENV_BIN -0 | $SORT_BIN -z'
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN BODY

  SHELL="$(which tcsh)" run --separate-stderr bash -c '
    "$ENV_BIN" -0 | "$SORT_BIN" -z > before
    "$FLOX_BIN" activate -c "$BODY" > after
    "$COMM_BIN" -z -3 before after | "$CUT_BIN" -z -d= -f1 | "$TR_BIN" -d "\t" | "$SORT_BIN" -uz | "$TR_BIN" "\0" "\n"
  '
  assert_success
  assert_output ""
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (zsh)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  SORT_BIN=$(command -v sort)
  COMM_BIN=$(command -v comm)
  TR_BIN=$(command -v tr)
  CUT_BIN=$(command -v cut)
  BODY='eval "$($FLOX_BIN deactivate --print-script)"; $ENV_BIN -0 | $SORT_BIN -z'
  export ENV_BIN SORT_BIN COMM_BIN TR_BIN CUT_BIN BODY

  # `grep -v BASH_FUNC_` drops bats' own exported helper functions, which
  # leak through the env capture in this path and are not flox behavior.
  FLOX_SHELL="zsh" run --separate-stderr \
    env "${FLOX_COLD_START_UNSET[@]}" bash -c '
    "$ENV_BIN" -0 | "$SORT_BIN" -z > before
    "$FLOX_BIN" activate -c "$BODY" > after
    "$COMM_BIN" -z -3 before after | "$CUT_BIN" -z -d= -f1 | "$TR_BIN" -d "\t" | "$SORT_BIN" -uz | "$TR_BIN" "\0" "\n" | grep -v BASH_FUNC_
  '
  assert_success
  assert_output - <<EOF
FLOX_SHELL
FLOX_VERSION
OLDPWD
SHLVL
SSL_CERT_FILE
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
}
