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

@test "hook-only: bash preserves prompt commands and manages directory activation" {
  hook_only_project_setup
  run --separate-stderr bash --noprofile --norc -ic '
    set -e
    export FLOX_SHELL=$(command -v bash)
    user_prompt() { :; }
    user_tail() { :; }
    PROMPT_COMMAND=(user_prompt user_tail)
    eval "$($FLOX_BIN activate --hook-only)"
    eval "$($FLOX_BIN activate --hook-only)"
    [[ ${#PROMPT_COMMAND[@]} == 2 ]]
    [[ ${PROMPT_COMMAND[0]} == "_flox_hook;user_prompt" ]]
    [[ ${PROMPT_COMMAND[1]} == user_tail ]]
    [[ -z ${FLOX_ENV-} ]]
    cd "$PROJECT_DIR"
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    cd child
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    $FLOX_BIN deactivate
    _flox_hook
    [[ -z ${FLOX_ENV-} ]]
    _flox_hook
    [[ -z ${FLOX_ENV-} ]]
    cd "$OUTSIDE_DIR"
    _flox_hook
    cd "$PROJECT_DIR"
    _flox_hook
    [[ $FLOX_ENV_PROJECT == "$PROJECT_DIR" ]]
    cd "$OUTSIDE_DIR"
    _flox_hook
    [[ -z ${FLOX_ENV-} ]]
    [[ ${#PROMPT_COMMAND[@]} == 2 ]]
    echo complete
  '
  assert_success
  assert_output --partial complete
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

@test "hook-only: tcsh retains replaced and extended aliases across activation" {
  hook_only_project_setup
  export HOOK_ONLY_USER_LOG="$BATS_TEST_TMPDIR/user-hooks"
  export HOOK_ONLY_USER_SCRIPT="$BATS_TEST_TMPDIR/user-hook"
  # Tcsh ignores redirection in its special aliases; the helper writes the log.
  cat > "$HOOK_ONLY_USER_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$HOOK_ONLY_USER_LOG"
EOF
  chmod +x "$HOOK_ONLY_USER_SCRIPT"
  cat > "$BATS_TEST_TMPDIR/commands.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
alias precmd '"$HOOK_ONLY_USER_SCRIPT" prompt'
alias cwdcmd '"$HOOK_ONLY_USER_SCRIPT" directory'
eval "`$FLOX_BIN activate --hook-only`"
eval "`$FLOX_BIN activate --hook-only`"
alias precmd '"$HOOK_ONLY_USER_SCRIPT" updated-prompt'
alias cwdcmd '"$HOOK_ONLY_USER_SCRIPT" updated-directory'
eval "`$FLOX_BIN activate --hook-only`"
alias _test_append '"$HOOK_ONLY_USER_SCRIPT" appended'
alias cwdcmd "`alias cwdcmd`; _test_append"
eval "`$FLOX_BIN activate --hook-only`"
eval "`$FLOX_BIN activate --hook-only`"
if ( $?FLOX_ENV ) exit 1
cd "$PROJECT_DIR"
if ( ! $?FLOX_ENV_PROJECT ) exit 2
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 3
(cd "$OUTSIDE_DIR")
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 4
cd child
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 5
cd "$OUTSIDE_DIR"
if ( $?FLOX_ENV ) exit 6
echo complete
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/commands.tcsh"
  assert_success
  assert_output --partial complete
  # The forked cd also runs the user's own cwdcmd, but never Flox's callback.
  run grep -c '^updated-directory$' "$HOOK_ONLY_USER_LOG"
  assert_output 4
  run grep -c '^appended$' "$HOOK_ONLY_USER_LOG"
  assert_output 4
  run tail -n 1 "$HOOK_ONLY_USER_LOG"
  assert_output updated-prompt
}

@test "hook-only: tcsh profiles can save and restore callbacks across activation" {
  hook_only_project_setup
  run "$FLOX_BIN" edit --dir "$PROJECT_DIR" -f - <<'EOF'
schema-version = "1.13.0"

[profile]
tcsh = '''
set hook_only_saved_precmd="`alias precmd`"
set hook_only_saved_cwdcmd="`alias cwdcmd`"
alias precmd 'setenv HOOK_ONLY_PROMPT project'
alias cwdcmd 'setenv HOOK_ONLY_DIRECTORY project'
'''

[profile.deactivate]
tcsh = '''
alias precmd "$hook_only_saved_precmd"
alias cwdcmd "$hook_only_saved_cwdcmd"
unset hook_only_saved_precmd hook_only_saved_cwdcmd
'''
EOF
  assert_success
  cat > "$BATS_TEST_TMPDIR/restore.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
alias precmd 'setenv HOOK_ONLY_PROMPT base'
alias cwdcmd 'setenv HOOK_ONLY_DIRECTORY base'
eval "`$FLOX_BIN activate --hook-only`"
cd "$PROJECT_DIR"
if ( ! $?FLOX_ENV_PROJECT ) exit 1
if ( "$HOOK_ONLY_PROMPT" != project ) exit 2
cd child
if ( "$HOOK_ONLY_DIRECTORY" != project ) exit 3
cd "$OUTSIDE_DIR"
if ( $?FLOX_ENV ) exit 4
if ( "$HOOK_ONLY_PROMPT" != base ) exit 5
cd .
if ( "$HOOK_ONLY_DIRECTORY" != base ) exit 6
eval "`$FLOX_BIN activate --hook-only`"
if ( "$HOOK_ONLY_PROMPT" != base ) exit 7
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/restore.tcsh"
  assert_success
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

@test "hook-only: tcsh replaces the previous native registration without chaining it" {
  hook_only_project_setup
  export HOOK_ONLY_LEGACY_BIN="$BATS_TEST_TMPDIR/old-flox"
  cat > "$HOOK_ONLY_LEGACY_BIN" <<'EOF'
#!/usr/bin/env bash
printf 'setenv HOOK_ONLY_LEGACY_RAN yes;\n'
EOF
  chmod +x "$HOOK_ONLY_LEGACY_BIN"
  cat > "$BATS_TEST_TMPDIR/legacy.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
# Migrate pure native registrations first, then ones with appended user commands.
alias precmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $HOOK_ONLY_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit'
alias cwdcmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $HOOK_ONLY_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit'
eval "`$FLOX_BIN activate --hook-only`"
alias precmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $HOOK_ONLY_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit; echo "USER_PROMPT:$cwd"'
alias cwdcmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $HOOK_ONLY_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit; echo "USER_DIRECTORY:`pwd`"'
eval "`$FLOX_BIN activate --hook-only`"
eval "`$FLOX_BIN activate --hook-only`"
unsetenv HOOK_ONLY_LEGACY_RAN
if ( $?HOOK_ONLY_LEGACY_RAN ) exit 1
cd "$PROJECT_DIR"
if ( $?HOOK_ONLY_LEGACY_RAN ) exit 2
if ( ! $?FLOX_ENV_PROJECT ) exit 3
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 4
cd child
cd "$OUTSIDE_DIR"
if ( $?FLOX_ENV ) exit 5
if ( $?_FLOX_MIGRATION_CODE || $?_FLOX_LEGACY_ALIAS ) exit 6
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/legacy.tcsh"
  assert_success
  assert_output --partial "USER_PROMPT:$OUTSIDE_DIR"
  assert_output --partial "USER_DIRECTORY:$PROJECT_DIR/child"
}

@test "hook-only: bash preserves prompt callbacks and evaluates the starting directory at its first prompt" {
  hook_only_project_setup
  cd "$PROJECT_DIR" || return
  export HOOK_ONLY_BASHRC="$BATS_TEST_TMPDIR/bashrc"
  cat > "$HOOK_ONLY_BASHRC" <<'EOF'
user_prompt() { printf 'USER:%s\n' "$?"; }
if [[ $HOOK_ONLY_PROMPT_MODE == array ]]; then
  PROMPT_COMMAND=(user_prompt)
else
  PROMPT_COMMAND=user_prompt
fi
eval "$($FLOX_BIN activate --hook-only)"
eval "$($FLOX_BIN activate --hook-only)"
[[ -z ${FLOX_ENV-} ]] || exit 99
PS1='HOOK_ONLY> '
EOF
  cat > "$BATS_TEST_TMPDIR/first-prompt.exp" <<'EOF'
set timeout 30
spawn $env(FLOX_SHELL) --noprofile --rcfile $env(HOOK_ONLY_BASHRC) -i
expect {
  "HOOK_ONLY> " {}
  timeout { exit 1 }
  eof { exit 2 }
}
send {printf 'PROJECT:%s\n' "$FLOX_ENV_PROJECT"}
send "\r"
expect {
  -exact "PROJECT:$env(PROJECT_DIR)\r\n" {}
  timeout { exit 3 }
  eof { exit 4 }
}
expect "HOOK_ONLY> "
send "(exit 42)\r"
expect {
  -exact "USER:42\r\n" {}
  timeout { exit 5 }
  eof { exit 6 }
}
send "exit\r"
expect eof
EOF
  # Bash 3 only evaluates array element zero; test it where the host provides it.
  for shell in "$(command -v bash)" /bin/bash; do
    [ -x "$shell" ] || continue
    export FLOX_SHELL="$shell"
    for mode in scalar array; do
      export HOOK_ONLY_PROMPT_MODE="$mode"
      run expect "$BATS_TEST_TMPDIR/first-prompt.exp"
      assert_success
    done
  done
}
