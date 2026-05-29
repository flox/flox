#!/usr/bin/env tcsh
# tcsh state-dump helper for deactivate-state.bats.
#
# Snapshots tcsh shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single tcsh
# process so the diff catches anything activate/deactivate failed to
# restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR
#   _ALLOWED_LEAKS_RE, _TEST_HARNESS_NOISE_RE
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

# Pre-define vars that the tcsh deactivate generator references after
# tearing them down. Without a placeholder value here, activate's
# snapshot has nothing to restore them to, so deactivate emits
# `unsetenv FLOX_ENV` (etc.), and the *same* deactivate script then
# references `$FLOX_ENV` in `profile-scripts --env-dirs $FLOX_ENV:q`
# and `source set-prompt.tcsh` (which tests `"$FLOX_PROMPT_ENVIRONMENTS"
# != ""`). tcsh treats `$VAR` on an undefined name as a fatal error,
# unlike bash. Pre-seeding to empty keeps the snapshot symmetric (the
# placeholders appear in both pre and post) and lets the deactivate
# script run to completion. A proper fix belongs in the tcsh
# deactivate generator — track separately.
setenv FLOX_ENV ""
setenv FLOX_PROMPT_ENVIRONMENTS ""

# tcsh-internal volatiles and this script's own helpers, filtered from
# the snapshot.
#   status / argv / _        — auto-vars, change between calls
#   cwd / dirstack / cdpath  — directory tracking
#   tcsh / tcsh_version / version
#                            — tcsh build identification
#   tty / loginsh / shlvl / owd
#                            — process state
#   _TCSH_INTERNAL_VAR_RE / _TCSH_ALLOWED_LEAKS_RE
#                            — this script's own shell vars
set _TCSH_INTERNAL_VAR_RE='^(status|argv|_|cwd|dirstack|cdpath|tcsh|tcsh_version|version|tty|loginsh|shlvl|owd|_TCSH_INTERNAL_VAR_RE|_TCSH_ALLOWED_LEAKS_RE)$'

# tcsh-specific intentional leaks not covered by the cross-shell list.
#   precmd / cwdcmd          — activate.tcsh registers these aliases to
#                              re-run `flox hook-env` on prompt and
#                              directory change. Not removed by
#                              deactivate; tracked separately.
#   _already_sourced_args    — activate-local helper var
#                              (`set _already_sourced_args = ();`); not
#                              cleaned up.
set _TCSH_ALLOWED_LEAKS_RE='^(precmd|cwdcmd|_already_sourced_args)$'

# tcsh has no brace groups and `( … )` requires a single command on
# one line — so the dump body is written as a series of per-line
# appends. `:` would be ideal as a noop initializer but tcsh treats
# `:` differently from sh; use `echo -n > $1` to truncate instead.

# Pre snapshot
echo -n '' > $1
echo '=== ALIASES ===' >> $1
alias | awk '{print $1}' | sort | grep -vE "$_TCSH_ALLOWED_LEAKS_RE" >> $1
echo '=== SHELL_VARS ===' >> $1
# tcsh `set` prints `name<TAB>value`; awk extracts the first whitespace
# field. Multi-line values (rare) may be slightly mis-handled — if
# we hit that in practice, switch to a `printvars` builtin call.
set | awk '{print $1}' | sort -u | grep -vE "$_TCSH_INTERNAL_VAR_RE" | grep -vE "$_TCSH_ALLOWED_LEAKS_RE" | grep -vE "$_ALLOWED_LEAKS_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $1
echo '=== ENV_VARS ===' >> $1
# `setenv` prints `VAR=value`; cut at the first `=`.
setenv | awk -F= '{print $1}' | sort -u | grep -vE "$_ALLOWED_LEAKS_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $1

eval "`$FLOX_BIN activate -d $PROJECT_DIR`"
eval "`$FLOX_BIN deactivate --print-script`"

# Post snapshot
echo -n '' > $2
echo '=== ALIASES ===' >> $2
alias | awk '{print $1}' | sort | grep -vE "$_TCSH_ALLOWED_LEAKS_RE" >> $2
echo '=== SHELL_VARS ===' >> $2
set | awk '{print $1}' | sort -u | grep -vE "$_TCSH_INTERNAL_VAR_RE" | grep -vE "$_TCSH_ALLOWED_LEAKS_RE" | grep -vE "$_ALLOWED_LEAKS_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $2
echo '=== ENV_VARS ===' >> $2
setenv | awk -F= '{print $1}' | sort -u | grep -vE "$_ALLOWED_LEAKS_RE" | grep -vE "$_TEST_HARNESS_NOISE_RE" >> $2
