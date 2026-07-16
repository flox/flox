#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox deactivate' subcommand.
# We are especially interested in ensuring that the deactivation script properly
# restores environment variables and cleans up after activation.
#
# Scope: this file checks *exported environment* restoration — env-value
# diffs across the in-place, subshell, and interactive (pty) modes, plus
# prompt and individual user-set variables. Full shell-state restoration by
# *name* (functions, aliases, shell options, non-exported vars — the surface
# `env` cannot see) lives in the companion shell-state-restoration.bats. The two
# keep overlapping noise/allow filters; when a shared name (NIX_SSL_CERT_FILE,
# PATH_LOCALE, _activate_d, _flox_activate_tracer) is reclassified in one
# file, update the other.
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
  home_setup test
  user_dotfiles_setup
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

# Assert that a user-controlled variable keeps its user-set value across an
# activate/deactivate round-trip. flox may read such variables but must not
# overwrite a value the user set, and must restore it on deactivate.
#
# $1: variable name
# $2: the user-set value
#
# Runs under zsh (FLOX_SHELL=zsh selects the shell; for the FLOX_SHELL case the
# variable under test and the shell selector coincide, which is fine). The
# complementary "must not be leaked when the user did not set it" guarantee is
# already covered by the in-place env-diff tests below, whose expected blocks
# would fail if such a variable started surviving deactivate.
assert_user_var_preserved() {
  local var="$1" value="$2"
  run --separate-stderr env FLOX_SHELL=zsh "$var=$value" zsh -c "
    eval \"\$(\$FLOX_BIN activate --print-script)\"
    echo \"during:\$$var\"
    eval \"\$(\$FLOX_BIN deactivate --print-script)\"
    echo \"after:\$$var\"
  "
  assert_success
  assert_line "during:$value"
  assert_line "after:$value"
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q
    eval "`$FLOX_BIN deactivate --print-script-from-env`"
    unsetenv _FLOX_INVOCATION_TYPES_WIRE
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q
    eval "`$FLOX_BIN deactivate --print-script-from-env`"
    unsetenv _FLOX_INVOCATION_TYPES_WIRE
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval "$("$FLOX_BIN" deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
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
    eval ($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")
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
    setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q
    eval "`$FLOX_BIN deactivate --print-script-from-env`"
    unsetenv _FLOX_INVOCATION_TYPES_WIRE
    echo "<after>$prompt</after>"
  '
  assert_success
  assert_prompt_round_trip "$output"
}

# ---------------------------------------------------------------------------- #
# end prompt tests
# ---------------------------------------------------------------------------- #

# User-controlled variables: flox may read these but must not overwrite a value
# the user set, and must restore it on deactivate. We assert the user's value
# survives the round-trip; the complementary "not leaked when the user did not
# set it" guarantee is already covered by the in-place env-diff tests below
# (their expected blocks would fail if one of these started surviving).
#
# NOTE: NIX_SSL_CERT_FILE is NOT yet in this set — it is set unconditionally,
# leaks, and overwrites a user value. Add it here once fixed; see
# NIX_SSL_CERT_FILE-findings.md.

# bats test_tags=deactivate
@test "deactivate preserves a user-set FLOX_SHELL (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  assert_user_var_preserved FLOX_SHELL zsh
}

# bats test_tags=deactivate
@test "deactivate preserves a user-set SSL_CERT_FILE (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  assert_user_var_preserved SSL_CERT_FILE /user/cert
}

# ---------------------------------------------------------------------------- #

# Full-environment diff tests. These capture `env` before activation and
# after deactivation, then assert the set of vars whose value changed
# (or that were added/removed) matches the inline expected list. Treat
# the expected list as a TODO -- when a fix lands, shrink it.
#

# These variables could be already set in the environment where the test is run,
# and if they're unset unconditionally by activate, they'll interfere with the
# test
# TODO: investigate if all of these are necessary
FLOX_COLD_START_UNSET=(
  -u _FLOX_HOOK_DIFF
  -u FLOX_VERSION
  -u _FLOX_HOOK_SAVE_FPATH
  -u _activate_d
  # Exported by an outer activation (e.g. when the test suite itself runs
  # inside one) and unset by the deactivate flow, so leaked values surface
  # as spurious env-diff records.
  -u _FLOX_PROMPT_HOOK_VERSION
  # The old name was exported by released flox; the new one is never
  # exported, but scrub both defensively.
  -u _FLOX_INVOCATION_TYPE
  -u _FLOX_INVOCATION_TYPES
  # A leaked auto-activate flag makes the prompt hook auto-activate inside
  # interactive test sessions, which also surfaces in the env diff.
  -u FLOX_FEATURES_AUTO_ACTIVATE
  # State vars maintained by the prompt hook in any outer shell that
  # activated with auto-activate enabled (e.g. a developer's own terminal).
  # _FLOX_HOOK_FIRED is only set by older flox versions but can still leak
  # from an outer shell running one.
  -u _FLOX_AUTO_ACTIVATED_ENVIRONMENTS
  -u _FLOX_SUPPRESSED_ENVIRONMENTS
  -u _FLOX_HOOK_FIRED
)

# Wrapper for the cold-start env prefix. In addition to unsetting the
# vars in FLOX_COLD_START_UNSET, it overrides PATH with a copy that has
# empty entries stripped. Some shells (fish in particular) rewrite empty
# PATH entries to the literal `.` on subshell launch, which would surface
# PATH as a value-changed record in the env-diff even though no real leak
# occurred.
flox_cold_start() {
  local p="$PATH"
  while [[ "$p" == *::* ]]; do p="${p//::/:}"; done
  p="${p#:}"
  p="${p%:}"
  env "${FLOX_COLD_START_UNSET[@]}" PATH="$p" "$@"
}

# Print the value of $2 from the null-delimited env dump file $1.
# Exits 0 with the value on stdout if found, 1 if the var is unset.
# Preserves multi-line values (awk handles NUL records natively).
env_var_value() {
  awk -v RS='\0' -v var="$2" '
    {
      eq = index($0, "=")
      if (eq > 0 && substr($0, 1, eq - 1) == var) {
        printf "%s", substr($0, eq + 1)
        found = 1
        exit
      }
    }
    END { if (!found) exit 1 }
  ' "$1"
}

# Print the set of env vars whose value changed (or were added/removed)
# between two raw null-delimited env dumps. The diff'd names go to stdout
# (one per line, sorted ASCII). A pretty per-var BEFORE vs AFTER block
# also goes to stderr for debugging when the assertion fails. Tests
# should capture stdout manually so stderr flows through to bats:
#
#   output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
#
# With `run --separate-stderr` the verbose block would be captured into
# $stderr and hidden by bats on failure.
diff_env_dumps() {
  local before="${1:?}"
  shift
  local after="${1:?}"
  shift
  local IFS='|'

  local noise
  # Variables we can ignore from the tests but that we're not responsible for.
  # TODO: not all entries below are truly noise.
  # TODO: user_dotfiles_setup may be introducing some noise that we haven't
  # accounted for
  case "$OSTYPE" in
    darwin*)
      noise=(
        BASH_FUNC_
        BUILDENV_NIX
        DYLD_LIBRARY_PATH
        LOGNAME
        NIX_SSL_CERT_FILE
        PATH_LOCALE
        REMOTEHOST
        _flox_activate_tracer
      )
      ;;
    *)
      noise=(
        BASH_FUNC_
        BUILDENV_NIX
        LD_LIBRARY_PATH
        LOCALE_ARCHIVE
        LOGNAME
        LS_COLORS
        NIX_SSL_CERT_FILE
        REMOTEHOST
        SSL_CERT_FILE
        _flox_activate_tracer
      )
      ;;
  esac

  # Variables that may be unset by activating and deactivating, even if they
  # were previously set in the environment.
  # Currently empty: vars previously listed here (_FLOX_HOOK_DIFF, FLOX_VERSION)
  # are now handled by FLOX_COLD_START_UNSET, which keeps them out of `before`
  # entirely so the BEFORE-only filter is unnecessary.
  ok_to_unset=()

  local noise_pattern="^(${noise[*]})"
  local ok_to_unset_pattern="^(${ok_to_unset[*]})="

  local names
  names=$(
    {
      # Records only in BEFORE -- strip OK_TO_UNSET names from this stream.
      LC_ALL=C comm -z -23 <(LC_ALL=C sort -z "$before") <(LC_ALL=C sort -z "$after") |
        LC_ALL=C grep -z -v -E "$ok_to_unset_pattern"
      # Records only in AFTER -- no OK_TO_UNSET filter.
      LC_ALL=C comm -z -13 <(LC_ALL=C sort -z "$before") <(LC_ALL=C sort -z "$after")
    } |
      cut -z -d= -f1 | tr -d "\t" | LC_ALL=C sort -uz | tr "\0" "\n" |
      grep -v -E "$noise_pattern"
  )

  if [[ -n "$names" ]]; then
    {
      printf -- '--- env diff (BEFORE vs AFTER) ---\n'
      local name b_val a_val
      while IFS= read -r name; do
        printf '%s\n' "$name"
        if b_val=$(env_var_value "$before" "$name"); then
          printf '  before: %s\n' "$b_val"
        else
          printf '  before: <unset>\n'
        fi
        if a_val=$(env_var_value "$after" "$name"); then
          printf '  after:  %s\n' "$a_val"
        else
          printf '  after:  <unset>\n'
        fi
      done <<<"$names"
    } >&2
    printf '%s\n' "$names"
  fi
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (bash)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  FLOX_SHELL="bash" run -0 flox_cold_start bash -c '
    "$ENV_BIN" -0 > "$BEFORE"
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    "$ENV_BIN" -0 > "$AFTER"
  '

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  refute_output
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  SHELL="$(which fish)" run -0 flox_cold_start fish -c '
    "$ENV_BIN" -0 > "$BEFORE"
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    "$ENV_BIN" -0 > "$AFTER"
  '

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  refute_output
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  SHELL="$(which tcsh)" run -0 flox_cold_start tcsh -c '
    "$ENV_BIN" -0 > "$BEFORE"
    eval "`$FLOX_BIN activate --print-script`"
    setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q
    eval "`$FLOX_BIN deactivate --print-script-from-env`"
    unsetenv _FLOX_INVOCATION_TYPES_WIRE
    "$ENV_BIN" -0 > "$AFTER"
  '

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  refute_output
}

# bats test_tags=activate,deactivate
@test "in-place deactivate env diff (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  FLOX_SHELL="zsh" run -0 flox_cold_start zsh -c '
    "$ENV_BIN" -0 > "$BEFORE"
    eval "$($FLOX_BIN activate --print-script)"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    "$ENV_BIN" -0 > "$AFTER"
  '

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  assert_output - <<EOF
SHLVL
EOF
}

# Subshell-mode counterparts: `flox activate -c "..."` runs the body in
# the activated subshell. The body deactivates in-place inside that
# subshell and dumps env, which is captured as `after`; `before` is the
# PARENT shell's pre-activation env.
# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (bash)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  COMMAND='eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"; $ENV_BIN -0'
  export ENV_BIN

  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  FLOX_SHELL="bash" flox_cold_start "$FLOX_BIN" activate -c "$COMMAND" > "$AFTER"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success

  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
PS1
SHLVL
SSL_CERT_FILE
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
PS1
SHLVL
SSL_CERT_DIR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  COMMAND='eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"; $ENV_BIN -0'
  export ENV_BIN

  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  SHELL="$(which fish)" flox_cold_start "$FLOX_BIN" activate -c "$COMMAND" > "$AFTER"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success

  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
SHELL
SSL_CERT_FILE
_
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
SHELL
SSL_CERT_DIR
_
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  COMMAND='setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`$FLOX_BIN deactivate --print-script-from-env`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; $ENV_BIN -0'
  export ENV_BIN

  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  SHELL="$(which tcsh)" flox_cold_start "$FLOX_BIN" activate -c "$COMMAND" > "$AFTER"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
FLOX_ORIG_HOME
FLOX_TCSH_INIT_SCRIPT
GROUP
HOST
HOSTTYPE
MACHTYPE
OSTYPE
SHELL
SHLVL
SSL_CERT_FILE
VENDOR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
FLOX_ORIG_HOME
FLOX_TCSH_INIT_SCRIPT
GROUP
HOST
HOSTTYPE
MACHTYPE
OSTYPE
SHELL
SHLVL
SSL_CERT_DIR
VENDOR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "subshell deactivate env diff (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  COMMAND='eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"; $ENV_BIN -0'
  export ENV_BIN

  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  FLOX_SHELL="zsh" flox_cold_start "$FLOX_BIN" activate -c "$COMMAND" > "$AFTER"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
FLOX_ORIG_ZDOTDIR
OLDPWD
PS1
SSL_CERT_FILE
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
FLOX_ORIG_ZDOTDIR
OLDPWD
PS1
SSL_CERT_DIR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# Interactive-mode counterparts: drive a real interactive `flox activate`
# session via expect (reusing activate-command.exp), running an intermediate
# shell at the prompt that deactivates in-place and dumps env to a file.
# The intermediate shell matters: deactivating at the top-level interactive
# prompt would just exit the session before we could capture anything.
# Like the in-place tests, this compares the parent's pre-activation env
# against the env captured after deactivate in the same shell -- but in an
# interactive context (rc files sourced, real ptys).
#
# Per-test setup: home_setup test + user_dotfiles_setup creates rc files
# that emit KNOWN_PROMPT so activate-command.exp can match the prompt and
# send commands. The in-place / subshell tests above use `bash -c` /
# `flox activate -c`, which don't source rc files, so they don't need this.

# bats test_tags=activate,deactivate
@test "interactive deactivate env diff (bash)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"


  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  # Absolute path to the intermediate shell: bare names aren't on PATH
  # inside the activated session (the test rc files reset it to BADPATH).
  SHELL_BIN=$(command -v bash)
  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  CMD="$SHELL_BIN -c 'eval \"\$(\$FLOX_BIN deactivate --print-script \"\$_FLOX_INVOCATION_TYPES\")\"; \$ENV_BIN -0 > \$AFTER'"
  FLOX_SHELL="bash" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR" "$CMD"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
NO_COLOR
PATH
SHLVL
SSL_CERT_FILE
TCLLIBPATH
TERM
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
NO_COLOR
PATH
SHLVL
SSL_CERT_DIR
TCLLIBPATH
TERM
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "interactive deactivate env diff (fish)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  SHELL_BIN=$(command -v fish)
  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  CMD="$SHELL_BIN -c 'eval \"\$(\$FLOX_BIN deactivate --print-script \"\$_FLOX_INVOCATION_TYPES\")\"; \$ENV_BIN -0 > \$AFTER'"
  FLOX_SHELL="fish" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR" "$CMD"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
NO_COLOR
PATH
SHLVL
SSL_CERT_FILE
TCLLIBPATH
TERM
_
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
NO_COLOR
PATH
SHLVL
SSL_CERT_DIR
TCLLIBPATH
TERM
_
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "interactive deactivate env diff (tcsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  SHELL_BIN=$(command -v tcsh)
  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  CMD="$SHELL_BIN -c 'if ( ! \$?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES; setenv _FLOX_INVOCATION_TYPES_WIRE \$_FLOX_INVOCATION_TYPES:q; eval \"\`\$FLOX_BIN deactivate --print-script-from-env\`\"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; \$ENV_BIN -0 > \$AFTER'"
  FLOX_SHELL="tcsh" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR" "$CMD"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  # macOS and Linux differ only in the SSL cert var (SSL_CERT_FILE vs
  # SSL_CERT_DIR).
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
FLOX_ORIG_HOME
FLOX_TCSH_INIT_SCRIPT
GROUP
HOST
HOSTTYPE
MACHTYPE
NO_COLOR
OSTYPE
PATH
SHLVL
SSL_CERT_FILE
TCLLIBPATH
TERM
VENDOR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
FLOX_ORIG_HOME
FLOX_TCSH_INIT_SCRIPT
GROUP
HOST
HOSTTYPE
MACHTYPE
NO_COLOR
OSTYPE
PATH
SHLVL
SSL_CERT_DIR
TCLLIBPATH
TERM
VENDOR
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# bats test_tags=activate,deactivate
@test "interactive deactivate exits the subshell via the prompt hook (tcsh)" {
  project_setup

  # Unlike the env-diff tests above, deactivate at the TOP-LEVEL interactive
  # prompt: the next prompt's hook consumes the action and exits the subshell.
  # Regression test: tcsh used to print "Faulty alias 'precmd' removed." and
  # stay in the subshell, because an `exit` unwinding out of the eval inside
  # the auto-fired precmd alias removes the alias instead of exiting (the
  # deactivation script now sets a flag the alias body acts on).
  FLOX_SHELL="tcsh" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/interactive-deactivate.exp" "$PROJECT_DIR"
  refute_output --partial "Faulty alias"
}

# bats test_tags=activate,deactivate
@test "interactive deactivate exits the subshell via the prompt hook (bash)" {
  project_setup
  FLOX_SHELL="bash" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/interactive-deactivate.exp" "$PROJECT_DIR"
}

# bats test_tags=activate,deactivate
@test "interactive deactivate exits the subshell via the prompt hook (fish)" {
  project_setup
  FLOX_SHELL="fish" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/interactive-deactivate.exp" "$PROJECT_DIR"
}

# bats test_tags=activate,deactivate
@test "interactive deactivate exits the subshell via the prompt hook (zsh)" {
  project_setup
  FLOX_SHELL="zsh" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/interactive-deactivate.exp" "$PROJECT_DIR"
}

# bats test_tags=activate,deactivate
@test "interactive deactivate env diff (zsh)" {
  project_setup
  "$FLOX_BIN" edit -f "$BATS_TEST_DIRNAME/activate/deactivate-vars.toml"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER

  SHELL_BIN=$(command -v zsh)
  flox_cold_start "$ENV_BIN" -0 > "$BEFORE"
  CMD="$SHELL_BIN -c 'eval \"\$(\$FLOX_BIN deactivate --print-script \"\$_FLOX_INVOCATION_TYPES\")\"; \$ENV_BIN -0 > \$AFTER'"
  FLOX_SHELL="zsh" run -0 \
    flox_cold_start expect "$TESTS_DIR/activate/activate-command.exp" "$PROJECT_DIR" "$CMD"

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  if [[ "$OSTYPE" == darwin* ]]; then
    assert_output - <<EOF
FLOX_ORIG_ZDOTDIR
FLOX_SAVE_ZSH_PS1
NO_COLOR
OLDPWD
PATH
PS1
SHLVL
SSL_CERT_FILE
TCLLIBPATH
TERM
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  else
    assert_output - <<EOF
FLOX_ORIG_ZDOTDIR
FLOX_SAVE_ZSH_PS1
NO_COLOR
OLDPWD
PATH
PS1
SHLVL
SSL_CERT_DIR
TCLLIBPATH
TERM
_FLOX_ACTIVATIONS_VERBOSITY
_FLOX_SUBSYSTEM_VERBOSITY
EOF
  fi
}

# ---------------------------------------------------------------------------- #
# Layered in-place activation / deactivation tests
#
# These verify that stacking two in-place activations and then performing two
# deactivations leaves the environment identical to the pre-activation state.
# The key invariant: _FLOX_HOOK_DIFF must be restored to its outer value (or
# unset, for the outermost activation) rather than unconditionally cleared on
# the first deactivation.
# ---------------------------------------------------------------------------- #

# bats test_tags=activate,deactivate
@test "layered in-place deactivate env diff (bash)" {
  export PROJECT_DIR1="${BATS_TEST_TMPDIR}/project1"
  export PROJECT_DIR2="${BATS_TEST_TMPDIR}/project2"
  mkdir -p "$PROJECT_DIR1" "$PROJECT_DIR2"
  "$FLOX_BIN" init -d "$PROJECT_DIR1"
  "$FLOX_BIN" init -d "$PROJECT_DIR2"

  ENV_BIN=$(command -v env)
  BEFORE="$BATS_TEST_TMPDIR/before"
  AFTER="$BATS_TEST_TMPDIR/after"
  export ENV_BIN BEFORE AFTER PROJECT_DIR1 PROJECT_DIR2

  FLOX_SHELL="bash" run -0 flox_cold_start bash -c '
    "$ENV_BIN" -0 > "$BEFORE"
    eval "$($FLOX_BIN activate -d "$PROJECT_DIR1" --print-script)"
    eval "$($FLOX_BIN activate -d "$PROJECT_DIR2" --print-script)"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    # After first deactivate: env2 removed, env1 still active
    echo "$_FLOX_ACTIVE_ENVIRONMENTS" | grep -qF "$PROJECT_DIR1" \
      || { echo "env1 not active after first deactivate: $_FLOX_ACTIVE_ENVIRONMENTS"; exit 1; }
    echo "$_FLOX_ACTIVE_ENVIRONMENTS" | grep -qvF "$PROJECT_DIR2" \
      || { echo "env2 still active after first deactivate: $_FLOX_ACTIVE_ENVIRONMENTS"; exit 1; }
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    "$ENV_BIN" -0 > "$AFTER"
  '

  output=$(diff_env_dumps "$BEFORE" "$AFTER"); status=$?
  assert_success
  refute_output

  wait_for_activations "$PROJECT_DIR1" || true
  wait_for_activations "$PROJECT_DIR2" || true
  rm -rf "$PROJECT_DIR1" "$PROJECT_DIR2"
  unset PROJECT_DIR
}

# ---------------------------------------------------------------------------- #
# Subshells: a nested shell inherits the activation's exported environment but
# its PID was never attached, so deactivating there must restore the
# environment without emitting a detach (DEV-159)
# ---------------------------------------------------------------------------- #

# Repro for DEV-159 with shell $1: activate in command mode, spawn a nested
# copy of the same shell, and deactivate inside it. $2 is the
# deactivate-and-observe script for the nested shell: it probes the manifest
# var FOO before and after eval'ing
# `flox deactivate --print-script "$_FLOX_INVOCATION_TYPES"` — the map
# variable is unset in a shell that performed no activation, so the nested
# shell passes an empty map exactly as its prompt hook would. FOO must go
# set → unset in the nested shell ("sub1:set" / "sub2:unset") while the
# parent's copy stays set, and the emitted script must not detach the
# parent's activation.
assert_subshell_deactivate_does_not_detach() {
  local shell="${1:?}" inner="${2:?}"

  project_setup
  cat << "EOF" | "$FLOX_BIN" edit -f -
version = 1

[vars]
FOO = "set"
EOF

  # Absolute paths: the nested shell is spawned from inside the activation,
  # where bare names may not resolve (user_dotfiles_setup resets PATH).
  SHELL_BIN="$(command -v "$shell")"
  INNER_SCRIPT="$BATS_TEST_TMPDIR/subshell-deactivate.$shell"
  echo "$inner" > "$INNER_SCRIPT"
  export SHELL_BIN INNER_SCRIPT

  FLOX_SHELL="$SHELL_BIN" run --separate-stderr "$FLOX_BIN" activate -d "$PROJECT_DIR" -c \
    '"$SHELL_BIN" "$INNER_SCRIPT" && echo "parent:$FOO"'
  assert_success
  refute_regex "$stderr" "is not attached to the activation"
  # The nested shell saw the inherited value, then restored its environment...
  assert_output --partial "sub1:set"
  assert_output --partial "sub2:unset"
  # ...without affecting the parent's.
  assert_output --partial "parent:set"
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' in a subshell does not detach the parent's activation (bash)" {
  assert_subshell_deactivate_does_not_detach bash '
    echo "sub1:$FOO"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    echo "sub2:${FOO:-unset}"
  '
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' in a subshell does not detach the parent's activation (fish)" {
  assert_subshell_deactivate_does_not_detach fish '
    echo "sub1:$FOO"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    set -q FOO; and echo "sub2:$FOO"; or echo "sub2:unset"
  '
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' in a subshell does not detach the parent's activation (tcsh)" {
  # Referencing an unset variable is a hard error in tcsh, and a one-line
  # `if` body must not contain `$` (it is substituted even when the
  # condition is false), so mirror the tcsh prompt hook: seed the map
  # variable empty, then pass it through the short-lived wire variable (a
  # JSON value cannot ride a tcsh backtick command line; see `tcsh_hook` in
  # flox-activations).
  assert_subshell_deactivate_does_not_detach tcsh "
    echo \"sub1:\$FOO\"
    if ( ! \$?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES
    setenv _FLOX_INVOCATION_TYPES_WIRE \$_FLOX_INVOCATION_TYPES:q
    eval \"\`\$FLOX_BIN deactivate --print-script-from-env\`\"
    unsetenv _FLOX_INVOCATION_TYPES_WIRE
    if ( \$?FOO ) echo 'sub2:set'
    if ( ! \$?FOO ) echo 'sub2:unset'
  "
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' in a subshell does not detach the parent's activation (zsh)" {
  assert_subshell_deactivate_does_not_detach zsh '
    echo "sub1:$FOO"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    echo "sub2:${FOO:-unset}"
  '
}

# Layer a default env and then a project env in an outer shell, spawn a
# nested $shell that re-activates the default env (the way a default-env rc
# file would), and deactivate once. The repeat activation must leave the
# `_FLOX_HOOK_DIFF` chain aligned with `_FLOX_ACTIVE_ENVIRONMENTS`, so that
# single deactivation pops the front (project) layer: its var is restored
# and it leaves the active list while the default env stays. $2 is the
# nested shell's script: repeat-activate, deactivate, then probe.
assert_subshell_repeat_keeps_deactivate_in_sync() {
  local shell="${1:?}" inner="${2:?}"

  project_setup
  cat << "EOF" | "$FLOX_BIN" edit -f -
version = 1

[vars]
PROJECT_VAR = "project_active"
EOF
  export DEFAULT_DIR="${BATS_TEST_TMPDIR}/default"
  mkdir -p "$DEFAULT_DIR"
  "$FLOX_BIN" init -d "$DEFAULT_DIR"

  SHELL_BIN="$(command -v "$shell")"
  INNER_SCRIPT="$BATS_TEST_TMPDIR/subshell-repeat.$shell"
  echo "$inner" > "$INNER_SCRIPT"
  export SHELL_BIN INNER_SCRIPT

  run --separate-stderr flox_cold_start "$SHELL_BIN" -c '
    eval "$($FLOX_BIN activate -d "$DEFAULT_DIR")"
    eval "$($FLOX_BIN activate -d "$PROJECT_DIR")"
    "$SHELL_BIN" "$INNER_SCRIPT"
  '
  assert_success
  refute_regex "$stderr" "is not attached to the activation"
  # One deactivate pops the project layer: its var is restored...
  assert_output --partial "sub:unset"
  # ...and it leaves the active list, while the default env remains.
  assert_regex "$output" "active:.*default"
  refute_regex "$output" "active:.*${PROJECT_NAME}"

  wait_for_activations "$DEFAULT_DIR" || true
  rm -rf "$DEFAULT_DIR"
  unset DEFAULT_DIR
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' pops the front layer after a repeat activation in a subshell (bash)" {
  assert_subshell_repeat_keeps_deactivate_in_sync bash '
    eval "$($FLOX_BIN activate -d "$DEFAULT_DIR")"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    echo "sub:${PROJECT_VAR:-unset}"
    echo "active:$_FLOX_ACTIVE_ENVIRONMENTS"
  '
}

# bats test_tags=deactivate:subshell
@test "'flox deactivate' pops the front layer after a repeat activation in a subshell (fish)" {
  assert_subshell_repeat_keeps_deactivate_in_sync fish '
    eval "$($FLOX_BIN activate -d "$DEFAULT_DIR")"
    eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPES")"
    set -q PROJECT_VAR; and echo "sub:$PROJECT_VAR"; or echo "sub:unset"
    echo "active:$_FLOX_ACTIVE_ENVIRONMENTS"
  '
}

# ---------------------------------------------------------------------------- #
# Stacked deactivation: an interactive activation layered with an in-place
# activation deactivates one layer per `flox deactivate`, and only the last
# one exits the shell
# ---------------------------------------------------------------------------- #

# Drive an interactive activation of PROJECT_DIR, layer an in-place
# activation of a second environment at the prompt, then run a plain
# `flox deactivate` twice, each followed by a probe that echoes
# "still_running". A deactivation takes effect at the next prompt, so the
# first one pops only the in-place layer and the session survives to run its
# probe; the second exits the activation subshell before its probe is read —
# exactly one "still_running" appears.
assert_stacked_deactivate_exits_only_last() {
  local shell="${1:?}"

  project_setup
  export PROJECT_DIR2="${BATS_TEST_TMPDIR}/project2"
  mkdir -p "$PROJECT_DIR2"
  "$FLOX_BIN" init -d "$PROJECT_DIR2"

  # Shell-syntax-specific eval of an in-place activation at the prompt.
  local inplace_cmd
  case "$shell" in
    bash | zsh | fish)
      inplace_cmd='eval "$('"$FLOX_BIN"' activate -d '"$PROJECT_DIR2"')"'
      ;;
    tcsh)
      inplace_cmd='eval "`'"$FLOX_BIN"' activate -d '"$PROJECT_DIR2"'`"'
      ;;
  esac

  FLOX_SHELL="$shell" run -0 flox_cold_start \
    expect "$TESTS_DIR/activate/stacked-deactivate.exp" \
    "$PROJECT_DIR" "$inplace_cmd"

  # The probe quote-splits its argument (still_"running"), so the echoed
  # input lines can't match; only probe output counts.
  assert_equal "$(grep -c 'still_running' <<< "$output")" 1

  wait_for_activations "$PROJECT_DIR2" || true
  rm -rf "$PROJECT_DIR2"
  unset PROJECT_DIR2
}

# bats test_tags=activate,deactivate
@test "stacked deactivate exits the shell only on the last layer (bash)" {
  assert_stacked_deactivate_exits_only_last bash
}

# bats test_tags=activate,deactivate
@test "stacked deactivate exits the shell only on the last layer (fish)" {
  assert_stacked_deactivate_exits_only_last fish
}

# bats test_tags=activate,deactivate
@test "stacked deactivate exits the shell only on the last layer (tcsh)" {
  assert_stacked_deactivate_exits_only_last tcsh
}

# bats test_tags=activate,deactivate
@test "stacked deactivate exits the shell only on the last layer (zsh)" {
  assert_stacked_deactivate_exits_only_last zsh
}

# ---------------------------------------------------------------------------- #

# Create a default environment and append the eval of an in-place activation
# to the shell's rc file, so every new $shell activates it the way a
# default-env setup does.
rc_default_env_setup() {
  local shell="${1:?}"

  export DEFAULT_DIR="${BATS_TEST_TMPDIR}/default"
  mkdir -p "$DEFAULT_DIR"
  "$FLOX_BIN" init -d "$DEFAULT_DIR"
  MANIFEST_CONTENTS="$(cat << "EOF"
    version = 1

    [vars]
    DEFAULT_MARKER = "default_env_active"
EOF
  )"
  echo "$MANIFEST_CONTENTS" | "$FLOX_BIN" edit -d "$DEFAULT_DIR" -f -

  local rc_file
  case "$shell" in
    bash) rc_file=".bashrc.extra" ;;
    zsh) rc_file=".zshrc.extra" ;;
    fish) rc_file=".config/fish/config.fish.extra" ;;
    tcsh) rc_file=".tcshrc.extra" ;;
  esac
  case "$shell" in
    bash | zsh | fish)
      echo 'eval "$('"$FLOX_BIN"' activate -d '"$DEFAULT_DIR"')"' >> "$HOME/$rc_file"
      ;;
    tcsh)
      echo 'eval "`'"$FLOX_BIN"' activate -d '"$DEFAULT_DIR"'`"' >> "$HOME/$rc_file"
      ;;
  esac
}

rc_default_env_teardown() {
  wait_for_activations "$DEFAULT_DIR" || true
  rm -rf "$DEFAULT_DIR"
  unset DEFAULT_DIR
}

# Activate a project environment interactively from a shell where the
# default env is already active, so the rc file's eval is a repeat
# activation that must not add a layer: one `flox deactivate` exits the
# session, and the outer shell's default env activation still deactivates
# cleanly afterwards.
assert_rc_default_repeat_deactivate_exits() {
  local shell="${1:?}"

  project_setup
  rc_default_env_setup "$shell"

  run --separate-stderr flox_cold_start bash <(cat <<EOF
    set -euo pipefail
    _expect="\$(command -v expect)"
    eval "\$(FLOX_SHELL=bash "$FLOX_BIN" activate -d "$DEFAULT_DIR")"
    echo "outer:\$DEFAULT_MARKER"
    FLOX_SHELL="$shell" "\$_expect" "$TESTS_DIR/activate/rc-default-repeat-deactivate.exp" "$PROJECT_DIR" "default_env_active"
    eval "\$(FLOX_SHELL=bash "$FLOX_BIN" deactivate --print-script "\$_FLOX_INVOCATION_TYPES")"
    echo "outer2:\${DEFAULT_MARKER:-unset}"
EOF
)
  assert_success
  refute_regex "$output" "is not attached to the activation"
  refute_regex "$stderr" "is not attached to the activation"
  assert_line "outer:default_env_active"
  # Not assert_line: pty escape residue from the expect session can share
  # the line with the probe output.
  assert_output --partial "outer2:unset"

  rc_default_env_teardown
}

# bats test_tags=activate,deactivate
@test "deactivate exits a session nested on an already-active rc-file default env (bash)" {
  assert_rc_default_repeat_deactivate_exits bash
}

# bats test_tags=activate,deactivate
@test "deactivate exits a session nested on an already-active rc-file default env (fish)" {
  assert_rc_default_repeat_deactivate_exits fish
}

# bats test_tags=activate,deactivate
@test "deactivate exits a session nested on an already-active rc-file default env (tcsh)" {
  assert_rc_default_repeat_deactivate_exits tcsh
}

# bats test_tags=activate,deactivate
@test "deactivate exits a session nested on an already-active rc-file default env (zsh)" {
  assert_rc_default_repeat_deactivate_exits zsh
}
