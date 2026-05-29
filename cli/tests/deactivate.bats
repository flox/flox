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
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (fish)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (tcsh)" {
  skip "tcsh fails due to FLOX_PROMPT_ENVIRONMENTS undefined variable issue"
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "`$FLOX_BIN deactivate --print-script`"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate restores environment variables (zsh)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
    echo "after:$TEST_VAR"
  '
  assert_success
  assert_line "during:modified"
  assert_line "after:original"
}

# bats test_tags=deactivate
@test "deactivate unsets added variables (bash)" {
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
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
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
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
  skip "tcsh fails due to FLOX_PROMPT_ENVIRONMENTS undefined variable issue"
  project_setup
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "`$FLOX_BIN deactivate --print-script`"
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
  export FLOX_FEATURES_AUTO_ACTIVATE=true
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
    eval "$($FLOX_BIN deactivate --print-script)"
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
  export FLOX_FEATURES_AUTO_ACTIVATE=true

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
    eval "$("$FLOX_BIN" deactivate --print-script)"
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
    eval "$("$FLOX_BIN" deactivate --print-script)"
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
    eval ($FLOX_BIN deactivate --print-script)
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
    eval "`$FLOX_BIN deactivate --print-script`"
    echo "<after>$prompt</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# ---------------------------------------------------------------------------- #
# end prompt tests
# ---------------------------------------------------------------------------- #
