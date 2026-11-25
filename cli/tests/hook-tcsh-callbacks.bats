#! /usr/bin/env bats

# Registering the tcsh prompt hook must keep the user's own precmd and cwdcmd
# aliases. On main, the hook is registered by activating an environment in
# place, so each test activates BASE_DIR to register it and checks that
# auto-activation of PROJECT_DIR still works through the chained aliases.

load test_support.bash

# bats file_tags=activate,hook,tcsh

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
  export BASE_DIR="${BATS_TEST_TMPDIR?}/base"
  mkdir -p "$BASE_DIR"
  BASE_DIR="$(realpath "$BASE_DIR")"
  run "$FLOX_BIN" init --bare --no-auto-setup --dir "$BASE_DIR"
  assert_success
  export PROJECT_DIR="${BATS_TEST_TMPDIR?}/project"
  mkdir -p "$PROJECT_DIR/child"
  PROJECT_DIR="$(realpath "$PROJECT_DIR")"
  run "$FLOX_BIN" init --bare --no-auto-setup --dir "$PROJECT_DIR"
  assert_success
  run "$FLOX_BIN" activate allow --dir "$PROJECT_DIR"
  assert_success
}

teardown() {
  common_test_teardown
}

# bats test_tags=hook:tcsh:callbacks
@test "tcsh: in-place activation keeps replaced and extended precmd and cwdcmd aliases" {
  export TCSH_CB_LOG="$BATS_TEST_TMPDIR/user-hooks"
  export TCSH_CB_SCRIPT="$BATS_TEST_TMPDIR/user-hook"
  # Tcsh ignores redirection in its special aliases; the helper writes the log.
  cat > "$TCSH_CB_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$TCSH_CB_LOG"
EOF
  chmod +x "$TCSH_CB_SCRIPT"
  cat > "$BATS_TEST_TMPDIR/commands.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
alias precmd '"$TCSH_CB_SCRIPT" prompt'
alias cwdcmd '"$TCSH_CB_SCRIPT" directory'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
eval "`$FLOX_BIN activate -d $BASE_DIR`"
alias precmd '"$TCSH_CB_SCRIPT" updated-prompt'
alias cwdcmd '"$TCSH_CB_SCRIPT" updated-directory'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
alias _test_append '"$TCSH_CB_SCRIPT" appended'
alias cwdcmd "`alias cwdcmd`; _test_append"
eval "`$FLOX_BIN activate -d $BASE_DIR`"
eval "`$FLOX_BIN activate -d $BASE_DIR`"
if ( "$FLOX_ENV_PROJECT" != "$BASE_DIR" ) exit 1
cd "$PROJECT_DIR"
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 3
(cd "$OUTSIDE_DIR")
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 4
cd child
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 5
cd "$OUTSIDE_DIR"
if ( "$FLOX_ENV_PROJECT" != "$BASE_DIR" ) exit 6
echo complete
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/commands.tcsh"
  assert_success
  assert_output --partial complete
  # The forked cd also runs the user's own cwdcmd, but never Flox's callback.
  run grep -c '^updated-directory$' "$TCSH_CB_LOG"
  assert_output 4
  run grep -c '^appended$' "$TCSH_CB_LOG"
  assert_output 4
  run tail -n 1 "$TCSH_CB_LOG"
  assert_output updated-prompt
}

# bats test_tags=hook:tcsh:callbacks
@test "tcsh: profiles can save and restore precmd and cwdcmd across auto-activation" {
  run "$FLOX_BIN" edit --dir "$PROJECT_DIR" -f - <<'EOF'
schema-version = "1.13.0"

[profile]
tcsh = '''
set tcsh_cb_saved_precmd="`alias precmd`"
set tcsh_cb_saved_cwdcmd="`alias cwdcmd`"
alias precmd 'setenv TCSH_CB_PROMPT project'
alias cwdcmd 'setenv TCSH_CB_DIRECTORY project'
'''

[profile.deactivate]
tcsh = '''
alias precmd "$tcsh_cb_saved_precmd"
alias cwdcmd "$tcsh_cb_saved_cwdcmd"
unset tcsh_cb_saved_precmd tcsh_cb_saved_cwdcmd
'''
EOF
  assert_success
  cat > "$BATS_TEST_TMPDIR/restore.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
alias precmd 'setenv TCSH_CB_PROMPT base'
alias cwdcmd 'setenv TCSH_CB_DIRECTORY base'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
cd "$PROJECT_DIR"
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 1
if ( "$TCSH_CB_PROMPT" != project ) exit 2
cd child
if ( "$TCSH_CB_DIRECTORY" != project ) exit 3
cd "$OUTSIDE_DIR"
if ( "$FLOX_ENV_PROJECT" != "$BASE_DIR" ) exit 4
if ( "$TCSH_CB_PROMPT" != base ) exit 5
cd .
if ( "$TCSH_CB_DIRECTORY" != base ) exit 6
eval "`$FLOX_BIN activate -d $BASE_DIR`"
if ( "$TCSH_CB_PROMPT" != base ) exit 7
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/restore.tcsh"
  assert_success
}

# A shell that registered the hook with an earlier Flox has the old native
# alias text in precmd and cwdcmd, possibly followed by the user's commands.
# bats test_tags=hook:tcsh:callbacks
@test "tcsh: in-place activation replaces the previous native registration without chaining it" {
  export TCSH_CB_LEGACY_BIN="$BATS_TEST_TMPDIR/old-flox"
  cat > "$TCSH_CB_LEGACY_BIN" <<'EOF'
#!/usr/bin/env bash
printf 'setenv TCSH_CB_LEGACY_RAN yes;\n'
EOF
  chmod +x "$TCSH_CB_LEGACY_BIN"
  cat > "$BATS_TEST_TMPDIR/legacy.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
# Migrate pure native registrations first, then ones with appended user commands.
alias precmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $TCSH_CB_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit'
alias cwdcmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $TCSH_CB_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
alias precmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $TCSH_CB_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit; echo "USER_PROMPT:$cwd"'
alias cwdcmd 'if ( ! $?_FLOX_INVOCATION_TYPES ) set _FLOX_INVOCATION_TYPES=""; setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q; eval "`exec $TCSH_CB_LEGACY_BIN hook-env --shell tcsh --shell-pid $$ --invocation-types-from-env --skip-in-forked-subshell`"; unsetenv _FLOX_INVOCATION_TYPES_WIRE; if ( $?_flox_exit ) exit; echo "USER_DIRECTORY:`pwd`"'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
eval "`$FLOX_BIN activate -d $BASE_DIR`"
unsetenv TCSH_CB_LEGACY_RAN
if ( $?TCSH_CB_LEGACY_RAN ) exit 1
cd "$PROJECT_DIR"
if ( $?TCSH_CB_LEGACY_RAN ) exit 2
if ( "$FLOX_ENV_PROJECT" != "$PROJECT_DIR" ) exit 4
cd child
cd "$OUTSIDE_DIR"
if ( "$FLOX_ENV_PROJECT" != "$BASE_DIR" ) exit 5
if ( $?_FLOX_MIGRATION_CODE || $?_FLOX_LEGACY_ALIAS ) exit 6
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/legacy.tcsh"
  assert_success
  # Printed at the prompt in PROJECT_DIR, which is only reached after migration.
  assert_output --partial "USER_PROMPT:$PROJECT_DIR"
  assert_output --partial "USER_DIRECTORY:$PROJECT_DIR/child"
}

# `alias` prints an ESC byte as the two characters `^[`, so replaying the
# captured text would print it at every prompt instead of setting the title.
# bats test_tags=hook:tcsh:callbacks
@test "tcsh: in-place activation drops a callback that alias cannot print faithfully" {
  printf '%s\n' \
    "setenv FLOX_SHELL \`which tcsh\`" \
    "alias precmd 'echo -n \"$(printf '\033')]0;TITLE$(printf '\007')\"'" \
    "alias cwdcmd 'echo -n \"\\033]2;KEEP\\007\"'" \
    'eval "`$FLOX_BIN activate -d $BASE_DIR`"' \
    'echo "SAVED_PRECMD:`alias _flox_saved_precmd_1`:"' \
    'echo "SAVED_CWDCMD:`alias _flox_saved_cwdcmd_1`:"' \
    "alias precmd 'echo -n \"ding$(printf '\007')\"'" \
    'eval "`$FLOX_BIN activate -d $BASE_DIR`"' \
    'echo "SAVED_BEL:`alias _flox_saved_precmd_2`:"' \
    'exit' > "$BATS_TEST_TMPDIR/lossy.tcsh"
  # Caret detection must not depend on locale collation. Unset PATH_LOCALE so
  # tcsh uses the host's locale data rather than the dev shell's, whose
  # en_US.UTF-8 collates in byte order. A locale the host lacks falls back to C.
  for locale in C en_US.UTF-8 C.UTF-8; do
    run --separate-stderr env -u PATH_LOCALE LC_ALL="$locale" tcsh -fi < "$BATS_TEST_TMPDIR/lossy.tcsh"
    assert_success
    assert_output --partial "SAVED_PRECMD::"
    assert_output --partial 'SAVED_CWDCMD:echo -n "\033]2;KEEP\007":'
    # BEL prints as ^G, which only the letter list catches.
    assert_output --partial "SAVED_BEL::"
    refute_output --partial "^["
  done

  # A recognized Flox wrapper is kept as it is, even with appended text that
  # `alias` cannot print faithfully, so the user's original callback survives.
  printf '%s\n' \
    "setenv FLOX_SHELL \`which tcsh\`" \
    "alias precmd 'echo ORIGINAL'" \
    'eval "`$FLOX_BIN activate -d $BASE_DIR`"' \
    "alias precmd \"\`alias precmd\`; echo -n '$(printf '\033')]0;TITLE'\"" \
    'eval "`$FLOX_BIN activate -d $BASE_DIR`"' \
    'echo "SAVED_PRECMD:`alias _flox_saved_precmd_1`:"' \
    'echo "PRECMD:`alias precmd`"' \
    'exit' > "$BATS_TEST_TMPDIR/lossy-wrapper.tcsh"
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/lossy-wrapper.tcsh"
  assert_success
  assert_output --partial "SAVED_PRECMD:echo ORIGINAL:"
  assert_output --regexp "PRECMD:.*_flox_saved_precmd_1; .*_flox_hook; echo -n"
}
