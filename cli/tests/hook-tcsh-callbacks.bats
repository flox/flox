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

# A tcsh error inside precmd makes tcsh remove the alias, which also held the
# hook. The hook must come back at the next prompt, so that a deactivation
# that only the prompt hook can perform still happens.
# bats test_tags=hook:tcsh:callbacks
@test "tcsh: keeps the prompt hook after a preserved precmd callback fails" {
  # Keep the test's own prompt, which the expect script waits for.
  export FLOX_SET_PROMPT=false
  export TCSH_CB_TCSHRC="$BATS_TEST_TMPDIR/tcshrc"
  cat > "$TCSH_CB_TCSHRC" <<'EOF'
setenv FLOX_SHELL `which tcsh`
set tcsh_cb_fragile=ok
alias precmd 'set tcsh_cb_status=$status; set tcsh_cb_copy="$tcsh_cb_fragile"'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
set prompt='TCSH_CB> '
EOF
  # Tcsh stops reading piped input at the first error, so drive a terminal.
  # A working callback must still see the previous command's status. No
  # directory changes after the callback fails: only the prompt callback can
  # then perform the deactivation.
  cat > "$BATS_TEST_TMPDIR/faulty-precmd.exp" <<'EOF'
set timeout 30
spawn $env(TCSH_CB_TCSH) -f
expect {
  "> " {}
  timeout { exit 1 }
  eof { exit 2 }
}
send "source \$TCSH_CB_TCSHRC\r"
expect "TCSH_CB> "
send "cd \$PROJECT_DIR\r"
expect "TCSH_CB> "
send "sh -c 'exit 7'\r"
expect "TCSH_CB> "
send "echo \"STATUS:\$tcsh_cb_status AT:\$FLOX_ENV_PROJECT:t\"\r"
expect {
  -exact "STATUS:7 AT:project\r\n" {}
  -re "STATUS:\[0-9\]+ AT:\[a-z\]+\r\n" { exit 3 }
  timeout { exit 4 }
  eof { exit 4 }
}
expect "TCSH_CB> "
send "unset tcsh_cb_fragile\r"
expect {
  -exact "Faulty alias 'precmd' removed." {}
  timeout { exit 5 }
  eof { exit 6 }
}
expect "TCSH_CB> "
send "\$FLOX_BIN deactivate\r"
expect {
  -exact "Undefined variable" { exit 7 }
  "TCSH_CB> " {}
}
send "echo \"AT:\$FLOX_ENV_PROJECT:t\"\r"
expect {
  -exact "AT:base\r\n" {}
  -exact "AT:project\r\n" { exit 8 }
  -exact "Undefined variable" { exit 9 }
  timeout { exit 10 }
  eof { exit 11 }
}
send "exit\r"
expect eof
EOF
  # The faulty-alias handling is tcsh's own; test the host's tcsh as well where it exists.
  for shell in "$(command -v tcsh)" /bin/tcsh; do
    [ -x "$shell" ] || continue
    export TCSH_CB_TCSH="$shell"
    run expect "$BATS_TEST_TMPDIR/faulty-precmd.exp"
    assert_success
  done
}

# The repair runs from periodic, so it must keep the user's own periodic, come
# back when something replaces periodic, and leave a deliberately removed
# precmd alone.
# bats test_tags=hook:tcsh:callbacks
@test "tcsh: the precmd repair keeps the user's periodic and respects a removed precmd" {
  export TCSH_CB_SCRIPT="$BATS_TEST_TMPDIR/user-hook"
  # Tcsh ignores redirection in its special aliases; the helper writes the log.
  cat > "$TCSH_CB_SCRIPT" <<'EOF'
#!/usr/bin/env bash
printf '%s\n' "$1" >> "$TCSH_CB_LOG"
EOF
  chmod +x "$TCSH_CB_SCRIPT"
  # Each log only receives what periodic runs after the registration before it.
  cat > "$BATS_TEST_TMPDIR/periodic.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
setenv TCSH_CB_LOG "$BATS_TEST_TMPDIR/before"
alias precmd 'echo USER_PRECMD'
alias periodic '"$TCSH_CB_SCRIPT" user-periodic'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
setenv TCSH_CB_LOG "$BATS_TEST_TMPDIR/after-first"
echo "WRAPPED:`alias periodic`"
alias periodic '"$TCSH_CB_SCRIPT" replaced-periodic'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
setenv TCSH_CB_LOG "$BATS_TEST_TMPDIR/after-second"
echo "REWRAPPED:`alias periodic`"
unalias precmd
true
echo "STILL_REMOVED:`alias precmd`:"
exit
EOF
  run --separate-stderr tcsh -fi < "$BATS_TEST_TMPDIR/periodic.tcsh"
  assert_success
  assert_output --regexp "WRAPPED:_flox_saved_periodic_[0-9]+; _flox_precmd_check"
  assert_output --regexp "REWRAPPED:_flox_saved_periodic_[0-9]+; _flox_precmd_check"
  assert_output --partial "STILL_REMOVED::"
  run grep -c '^user-periodic$' "$BATS_TEST_TMPDIR/after-first"
  assert_success
  run grep -c '^replaced-periodic$' "$BATS_TEST_TMPDIR/after-second"
  assert_success
  run grep -c '^user-periodic$' "$BATS_TEST_TMPDIR/after-second"
  assert_failure
}

# The wrapper restores $status before the user's callback. With printexitvalue
# set, tcsh reports a non-zero status after every builtin, so the restore must
# not print `Exit N` again at each prompt.
# bats test_tags=hook:tcsh:callbacks
@test "tcsh: the precmd wrapper does not repeat printexitvalue output" {
  cat > "$BATS_TEST_TMPDIR/exitvalue.tcsh" <<'EOF'
setenv FLOX_SHELL `which tcsh`
set printexitvalue
alias precmd 'true'
eval "`$FLOX_BIN activate -d $BASE_DIR`"
sh -c 'exit 7'
true
true
exit
EOF
  run tcsh -fi < "$BATS_TEST_TMPDIR/exitvalue.tcsh"
  assert_success
  run grep -c 'Exit 7' <<< "$output"
  assert_output 1
}
