#!/usr/bin/env tcsh
# tcsh state-dump helper for shell-state-restoration.bats.
#
# Snapshots tcsh shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single tcsh
# process so the diff catches anything activate/deactivate failed to
# restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR, _TEST_HARNESS_NOISE_RE
#
# Only true shell internals and host/harness noise are filtered here;
# intentional (allow-listed) leaks are left in and classified by
# _assert_state_restored in shell-state-restoration.bats.
#
# Usage:
#   tcsh dump.tcsh <pre.txt> <post.txt>
#
# tcsh has no functions — only aliases — so the dump shape differs
# from the other shells: alias names + shell-var names + env-var
# names. tcsh also has no inline functions, so the dump body is
# repeated inline at pre and post call sites rather than wrapped in a
# helper.

setenv LC_ALL C
setenv FLOX_FEATURES_AUTO_ACTIVATE true
setenv FLOX_SHELL `which tcsh`

# tcsh-internal volatiles and this script's own helpers, filtered from
# the snapshot.
#   status / argv / _        — auto-vars, change between calls
#   cwd / dirstack / cdpath  — directory tracking
#   tcsh / tcsh_version / version
#                            — tcsh build identification
#   tty / loginsh / shlvl / owd
#                            — process state
#   _TCSH_INTERNAL_VAR_RE    — this script's own shell var
set _TCSH_INTERNAL_VAR_RE='^(status|argv|_|cwd|dirstack|cdpath|tcsh|tcsh_version|version|tty|loginsh|shlvl|owd|_TCSH_INTERNAL_VAR_RE)$'

# tcsh has no brace groups and `( … )` requires a single command on
# one line — so the dump body is written as a series of per-line
# appends. `:` would be ideal as a noop initializer but tcsh treats
# `:` differently from sh; use `echo -n > $1` to truncate instead.

# Pre snapshot
echo -n '' > $1
echo '=== ALIASES ===' >> $1
alias | awk '{print $1}' | sort >> $1
echo '=== SHELL_VARS ===' >> $1
# tcsh `set` prints `name<TAB>value`; awk extracts the first whitespace
# field. Multi-line values (rare) may be slightly mis-handled — if
# we hit that in practice, switch to a `printvars` builtin call.
set | awk '{print $1}' | sort -u | grep -vE "$_TCSH_INTERNAL_VAR_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $1
echo '=== ENV_VARS ===' >> $1
# `setenv` prints `VAR=value`; cut at the first `=`.
setenv | awk -F= '{print $1}' | sort -u | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $1

eval "`$FLOX_BIN activate -d $PROJECT_DIR`"
# `--print-script-from-env` reads the invocation type map that `activate`
# recorded in `_FLOX_INVOCATION_TYPES` (here: in-place) through the
# short-lived wire variable — JSON cannot ride a tcsh backtick command
# line. See deactivate.bats.
setenv _FLOX_INVOCATION_TYPES_WIRE $_FLOX_INVOCATION_TYPES:q
eval "`$FLOX_BIN deactivate --print-script-from-env`"
unsetenv _FLOX_INVOCATION_TYPES_WIRE

# Post snapshot
echo -n '' > $2
echo '=== ALIASES ===' >> $2
alias | awk '{print $1}' | sort >> $2
echo '=== SHELL_VARS ===' >> $2
set | awk '{print $1}' | sort -u | grep -vE "$_TCSH_INTERNAL_VAR_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $2
echo '=== ENV_VARS ===' >> $2
setenv | awk -F= '{print $1}' | sort -u | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $2
