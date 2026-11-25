#! /usr/bin/env bats
# -*- mode: bats; -*-
# ============================================================================ #
#
# Test the `flox activate print-hook` command: the prompt hook it prints for
# each shell, the cases where it refuses, and auto-activation in a shell set up
# with it.
#
# ============================================================================ #

load test_support.bash

# bats file_tags=activate,hook,print-hook

# ---------------------------------------------------------------------------- #

setup() {
  common_test_setup
  enable_prompt_hook
  setup_isolated_flox
  export FLOX_AUTO_ACTIVATE=allowlist
  export OUTSIDE_DIR="${BATS_TEST_TMPDIR?}/outside"
  mkdir -p "$OUTSIDE_DIR"
  cd "$OUTSIDE_DIR" || return
}

teardown() {
  common_test_teardown
}

# An allowed project with a child directory, as a target for auto-activation.
print_hook_project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project"
  mkdir -p "$PROJECT_DIR/child"
  PROJECT_DIR="$(realpath "$PROJECT_DIR")"
  run "$FLOX_BIN" init --bare --no-auto-setup --dir "$PROJECT_DIR"
  assert_success
  run "$FLOX_BIN" activate allow --dir "$PROJECT_DIR"
  assert_success
}

# Point every Flox directory at a fresh location, make the current directory
# hold an environment that cannot be opened, and make any use of the Nix store
# fail. Generation that still succeeds is therefore independent of an
# environment, of Nix, and (with an empty token) of a FloxHub login.
print_hook_unusable_environment_setup() {
  export FLOX_CONFIG_DIR="$BATS_TEST_TMPDIR/fresh-config"
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/fresh-data"
  export FLOX_CACHE_DIR="$BATS_TEST_TMPDIR/fresh-cache"
  export FLOX_STATE_DIR="$BATS_TEST_TMPDIR/fresh-state"
  mkdir .flox
  echo 'invalid environment metadata' > .flox/env.json
}

# ---------------------------------------------------------------------------- #
# Generation
# ---------------------------------------------------------------------------- #

# bats test_tags=print-hook:generate:bash
@test "print-hook: bash generation needs no environment, account, or Nix" {
  print_hook_unusable_environment_setup
  run --separate-stderr env FLOX_SHELL="$(command -v bash)" \
    FLOX_FLOXHUB_TOKEN= NIX_REMOTE=disabled \
    "$FLOX_BIN" activate print-hook
  assert_success
  assert_output --partial "_FLOX_PROMPT_HOOK_VERSION"
  assert_output --partial "_flox_hook()"
}

# bats test_tags=print-hook:generate:zsh
@test "print-hook: zsh generation needs no environment, account, or Nix" {
  print_hook_unusable_environment_setup
  run --separate-stderr env FLOX_SHELL="$(command -v zsh)" \
    FLOX_FLOXHUB_TOKEN= NIX_REMOTE=disabled \
    "$FLOX_BIN" activate print-hook
  assert_success
  assert_output --partial "_FLOX_PROMPT_HOOK_VERSION"
  assert_output --partial "precmd_functions"
}

# bats test_tags=print-hook:generate:fish
@test "print-hook: fish generation needs no environment, account, or Nix" {
  print_hook_unusable_environment_setup
  run --separate-stderr env FLOX_SHELL="$(command -v fish)" \
    FLOX_FLOXHUB_TOKEN= NIX_REMOTE=disabled \
    "$FLOX_BIN" activate print-hook
  assert_success
  assert_output --partial "_FLOX_PROMPT_HOOK_VERSION"
  assert_output --partial "function _flox_hook --on-event fish_prompt"
}

# bats test_tags=print-hook:generate:tcsh
@test "print-hook: tcsh generation needs no environment, account, or Nix" {
  print_hook_unusable_environment_setup
  run --separate-stderr env FLOX_SHELL="$(command -v tcsh)" \
    FLOX_FLOXHUB_TOKEN= NIX_REMOTE=disabled \
    "$FLOX_BIN" activate print-hook
  assert_success
  assert_output --partial "_FLOX_PROMPT_HOOK_VERSION"
  assert_output --partial "alias precmd"
}

# ---------------------------------------------------------------------------- #
# Noninteractive shells
# ---------------------------------------------------------------------------- #
#
# Characterization tests: the hook registers in a noninteractive shell, as it
# does for in-place activation. They pin that behavior until flox/flox#4901
# decides whether it should change, and are expected to change with it.

# bats test_tags=print-hook:noninteractive:bash
@test "print-hook: bash registers the hook in a noninteractive shell" {
  export PRINT_HOOK_SCRIPT="$BATS_TEST_TMPDIR/hook-script"
  FLOX_SHELL="$(command -v bash)" "$FLOX_BIN" activate print-hook > "$PRINT_HOOK_SCRIPT"
  run bash -c 'unset _FLOX_PROMPT_HOOK_VERSION; source "$PRINT_HOOK_SCRIPT"; test -n "${_FLOX_PROMPT_HOOK_VERSION-}"'
  assert_success
  assert_output ''
}

# bats test_tags=print-hook:noninteractive:zsh
@test "print-hook: zsh registers the hook in a noninteractive shell" {
  export PRINT_HOOK_SCRIPT="$BATS_TEST_TMPDIR/hook-script"
  FLOX_SHELL="$(command -v zsh)" "$FLOX_BIN" activate print-hook > "$PRINT_HOOK_SCRIPT"
  run zsh -c 'unset _FLOX_PROMPT_HOOK_VERSION; source "$PRINT_HOOK_SCRIPT"; test -n "${_FLOX_PROMPT_HOOK_VERSION-}"'
  assert_success
  assert_output ''
}

# bats test_tags=print-hook:noninteractive:fish
@test "print-hook: fish registers the hook in a noninteractive shell" {
  export PRINT_HOOK_SCRIPT="$BATS_TEST_TMPDIR/hook-script"
  FLOX_SHELL="$(command -v fish)" "$FLOX_BIN" activate print-hook > "$PRINT_HOOK_SCRIPT"
  run fish --no-config -c 'set -e _FLOX_PROMPT_HOOK_VERSION; source "$PRINT_HOOK_SCRIPT"; set -q _FLOX_PROMPT_HOOK_VERSION'
  assert_success
  assert_output ''
}

# bats test_tags=print-hook:noninteractive:tcsh
@test "print-hook: tcsh registers the hook in a noninteractive shell" {
  export PRINT_HOOK_SCRIPT="$BATS_TEST_TMPDIR/hook-script"
  FLOX_SHELL="$(command -v tcsh)" "$FLOX_BIN" activate print-hook > "$PRINT_HOOK_SCRIPT"
  run tcsh -f -c 'unsetenv _FLOX_PROMPT_HOOK_VERSION; source "$PRINT_HOOK_SCRIPT"; if ( ! $?_FLOX_PROMPT_HOOK_VERSION ) exit 1'
  assert_success
  assert_output ''
}

# ---------------------------------------------------------------------------- #
# Refusals
# ---------------------------------------------------------------------------- #

# bats test_tags=print-hook:refuse
@test "print-hook: incompatible state emits no script" {
  run -1 --separate-stderr env _FLOX_PROMPT_HOOK_VERSION=0:true "$FLOX_BIN" activate print-hook
  assert_output ''
  assert_regex "$stderr" "incompatible version"
}

# bats test_tags=print-hook:refuse
@test "print-hook: prompt hook disabled by environment variable emits no script" {
  run -1 --separate-stderr env FLOX_DISABLE_HOOK=true "$FLOX_BIN" activate print-hook
  assert_output ''
  assert_regex "$stderr" "disable_hook = true"
}

# bats test_tags=print-hook:refuse
@test "print-hook: prompt hook disabled by config emits no script" {
  run "$FLOX_BIN" config --set disable_hook true
  assert_success
  run -1 --separate-stderr "$FLOX_BIN" activate print-hook
  assert_output ''
  assert_regex "$stderr" "disable_hook = true"
}

# ---------------------------------------------------------------------------- #
# Auto-activation in a shell set up with print-hook
# ---------------------------------------------------------------------------- #

# bats test_tags=print-hook:auto-activate:bash
@test "print-hook: bash auto-activates and deactivates on cd" {
  print_hook_project_setup
  # Drives `_flox_hook` by hand, as hook.bats does, instead of relying on
  # PROMPT_COMMAND firing in a noninteractive `bash -c`.
  run --separate-stderr bash -c '
    export FLOX_SHELL=$(command -v bash)
    eval "$($FLOX_BIN activate print-hook)"
    [[ -z ${FLOX_ENV-} ]] || exit 1
    cd "$PROJECT_DIR"
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 2
    cd child
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 3
    cd "$OUTSIDE_DIR"
    _flox_hook
    [[ -z ${FLOX_ENV-} ]] || exit 4
    echo complete
  '
  assert_success
  assert_output --partial complete
}

# `print-hook` is the outermost invocation, so it warns a logged-out user once,
# and the activations the hook launches afterwards stay quiet even though no
# environment is active to mark them as nested.
# bats test_tags=print-hook:auto-activate:logged-out
@test "print-hook: logged-out advisory is not repeated by hook-launched activations" {
  print_hook_project_setup
  unset FLOX_FLOXHUB_TOKEN
  run bash -c '
    export FLOX_SHELL=$(command -v bash)
    eval "$($FLOX_BIN activate print-hook)"
    cd "$PROJECT_DIR"
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 1
  '
  assert_success
  run grep -c "You are not logged in to FloxHub" <<< "$output"
  assert_output "1"
}

# bats test_tags=print-hook:auto-activate:zsh
@test "print-hook: zsh detects the caller and preserves hooks and fork ownership" {
  print_hook_project_setup
  run --separate-stderr zsh -fic '
    set -e
    unset FLOX_SHELL
    export SHELL=$(command -v bash)
    user_prompt() { :; }
    user_directory() { :; }
    precmd_functions=(user_prompt)
    chpwd_functions=(user_directory)
    eval "$($FLOX_BIN activate print-hook)"
    eval "$($FLOX_BIN activate print-hook)"
    [[ ${#precmd_functions} == 2 && ${#chpwd_functions} == 2 ]] || exit 1
    [[ -z ${FLOX_ENV-} ]] || exit 2
    cd "$PROJECT_DIR"
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 3
    (cd "$OUTSIDE_DIR")
    ignored=$(cd "$OUTSIDE_DIR" && pwd)
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 4
    cd child
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]] || exit 5
    cd "$OUTSIDE_DIR"
    [[ -z ${FLOX_ENV-} ]] || exit 6
    echo complete
  '
  assert_success
  assert_output --partial complete
}

# bats test_tags=print-hook:auto-activate:fish
@test "print-hook: fish preserves handlers and manages directory activation" {
  print_hook_project_setup
  run --separate-stderr fish --no-config -ic '
    set -gx FLOX_SHELL (command -v fish)
    function user_prompt --on-event fish_prompt; end
    $FLOX_BIN activate print-hook | source
    $FLOX_BIN activate print-hook | source
    functions -q user_prompt; or exit 1
    test (functions --handlers | string match -r "^fish_prompt _flox_hook\$" | count) -eq 1; or exit 2
    set -q FLOX_ENV; and exit 3
    emit fish_prompt
    cd "$PROJECT_DIR"
    test "$FLOX_ENV_PROJECT" = "$PROJECT_DIR"; or exit 4
    cd child
    test "$FLOX_ENV_PROJECT" = "$PROJECT_DIR"; or exit 5
    cd "$OUTSIDE_DIR"
    set -q FLOX_ENV; and exit 6
    echo complete
  '
  assert_success
  assert_output --partial complete
}

# bats test_tags=print-hook:auto-activate:fish
@test "print-hook: fish retains configured prompt-only directory evaluation" {
  print_hook_project_setup
  run "$FLOX_BIN" config --set auto_activate_fish_mode disable_arrow
  assert_success
  run --separate-stderr fish --no-config -ic '
    set -gx FLOX_SHELL (command -v fish)
    $FLOX_BIN activate print-hook | source
    test "$FLOX_AUTO_ACTIVATE_FISH_MODE" = disable_arrow; or exit 1
    emit fish_prompt
    cd "$PROJECT_DIR"
    set -q FLOX_ENV; and exit 2
    emit fish_prompt
    test "$FLOX_ENV_PROJECT" = "$PROJECT_DIR"; or exit 3
    cd "$OUTSIDE_DIR"
    emit fish_prompt
    not set -q FLOX_ENV; or exit 4
  '
  assert_success
}
