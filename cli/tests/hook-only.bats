#! /usr/bin/env bats

load test_support.bash

# bats file_tags=activate,hook,hook-only

setup_file() {
  common_file_setup
  export BATS_NO_PARALLELIZE_WITHIN_FILE=true
}

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

hook_only_project_setup() {
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project"
  mkdir -p "$PROJECT_DIR/child"
  PROJECT_DIR="$(realpath "$PROJECT_DIR")"
  run "$FLOX_BIN" init --bare --no-auto-setup --dir "$PROJECT_DIR"
  assert_success
  run "$FLOX_BIN" activate allow --dir "$PROJECT_DIR"
  assert_success
}

@test "hook-only: generation needs no environment, account, or Nix" {
  export FLOX_CONFIG_DIR="$BATS_TEST_TMPDIR/initialize-config"
  export FLOX_DATA_DIR="$BATS_TEST_TMPDIR/initialize-data"
  export FLOX_CACHE_DIR="$BATS_TEST_TMPDIR/initialize-cache"
  export FLOX_STATE_DIR="$BATS_TEST_TMPDIR/initialize-state"
  mkdir .flox
  echo 'invalid environment metadata' > .flox/env.json
  for shell in bash zsh fish tcsh; do
    run --separate-stderr env FLOX_SHELL="$(command -v "$shell")" \
      FLOX_FLOXHUB_TOKEN= NIX_REMOTE=disabled \
      "$FLOX_BIN" activate --hook-only
    assert_success
    [ -n "$output" ]
  done
}

@test "hook-only: noninteractive shells register hooks like in-place activation" {
  export HOOK_ONLY_SCRIPT="$BATS_TEST_TMPDIR/initialize"
  for shell in bash zsh fish tcsh; do
    FLOX_SHELL="$(command -v "$shell")" "$FLOX_BIN" activate --hook-only > "$HOOK_ONLY_SCRIPT"
    case "$shell" in
      bash|zsh)
        run "$shell" -c 'unset _FLOX_PROMPT_HOOK_VERSION; source "$HOOK_ONLY_SCRIPT"; test -n "${_FLOX_PROMPT_HOOK_VERSION-}"'
        ;;
      fish)
        run fish --no-config -c 'set -e _FLOX_PROMPT_HOOK_VERSION; source "$HOOK_ONLY_SCRIPT"; set -q _FLOX_PROMPT_HOOK_VERSION'
        ;;
      tcsh)
        run tcsh -f -c 'unsetenv _FLOX_PROMPT_HOOK_VERSION; source "$HOOK_ONLY_SCRIPT"; if ( ! $?_FLOX_PROMPT_HOOK_VERSION ) exit 1'
        ;;
    esac
    assert_success
    assert_output ''
  done
}

@test "hook-only: incompatible state and disabled integration emit no script" {
  run --separate-stderr env _FLOX_PROMPT_HOOK_VERSION=0:true "$FLOX_BIN" activate --hook-only
  assert_failure
  assert_output ''
  [[ "$stderr" == *"incompatible version"* ]]
  run --separate-stderr env FLOX_DISABLE_HOOK=true "$FLOX_BIN" activate --hook-only
  assert_failure
  assert_output ''
  [[ "$stderr" == *"disable_hook = true"* ]]
}

@test "hook-only: zsh detects the caller and preserves hooks and fork ownership" {
  hook_only_project_setup
  run --separate-stderr zsh -fic '
    set -e
    unset FLOX_SHELL
    export SHELL=$(command -v bash)
    user_prompt() { :; }
    user_directory() { :; }
    precmd_functions=(user_prompt)
    chpwd_functions=(user_directory)
    eval "$($FLOX_BIN activate --hook-only)"
    eval "$($FLOX_BIN activate --hook-only)"
    [[ ${#precmd_functions} == 2 && ${#chpwd_functions} == 2 ]]
    [[ -z ${FLOX_ENV-} ]]
    cd "$PROJECT_DIR"
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    (cd "$OUTSIDE_DIR")
    ignored=$(cd "$OUTSIDE_DIR" && pwd)
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    cd child
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    cd "$OUTSIDE_DIR"
    [[ -z ${FLOX_ENV-} ]]
    echo complete
  '
  assert_success
  assert_output --partial complete
}

@test "hook-only: fish preserves handlers and manages directory activation" {
  hook_only_project_setup
  run --separate-stderr fish --no-config -ic '
    set -gx FLOX_SHELL (command -v fish)
    function user_prompt --on-event fish_prompt; end
    $FLOX_BIN activate --hook-only | source
    $FLOX_BIN activate --hook-only | source
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

@test "hook-only: fish retains configured prompt-only directory evaluation" {
  hook_only_project_setup
  run "$FLOX_BIN" config --set auto_activate_fish_mode disable_arrow
  assert_success
  run --separate-stderr fish --no-config -ic '
    set -gx FLOX_SHELL (command -v fish)
    $FLOX_BIN activate --hook-only | source
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
