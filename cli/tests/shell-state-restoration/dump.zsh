#!/usr/bin/env zsh
# zsh state-dump helper for shell-state-restoration.bats.
#
# Snapshots zsh shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single zsh
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
#   dump.zsh <pre.txt> <post.txt>

emulate -L zsh
unsetopt verbose xtrace
set -u
export LC_ALL=C
export FLOX_FEATURES_AUTO_ACTIVATE=true
export FLOX_SHELL=$(command -v zsh)

# Pre-initialize compinit so the pre snapshot already contains the
# completion-system functions and parameters. Otherwise activate's own
# compinit call (in activate.d/zsh) loads them and they show up as
# spurious post-only entries. We point at a private dumpfile so we
# never write to the user's $XDG_CACHE_HOME.
autoload -Uz compinit
compinit -d "$BATS_TEST_TMPDIR/.zcompdump"

# zsh-internal volatile names and our own dump helpers, filtered from
# variable and function lists.
_ZSH_INTERNAL_VAR_RE='^(_|0|argv|argv0|status|pipestatus|EGID|EUID|GID|UID|LINENO|RANDOM|SECONDS|EPOCHSECONDS|EPOCHREALTIME|funcfiletrace|funcsourcetrace|funcstack|functrace|HISTCMD|COLUMNS|LINES|f|out|_ZSH_INTERNAL_VAR_RE|_DUMP_HELPERS_RE)$'

_DUMP_HELPERS_RE='^(_flox_dump)$'

_flox_dump() {
  local out=$1
  {
    echo '=== SETOPT ==='
    setopt | sort
    echo '=== FUNCTIONS ==='
    # Names only: zsh autoloads are lazy, so their bodies differ between
    # the "shadow" entry (`# undefined` + `builtin autoload -XUz`) and
    # the fully-resolved entry that exists after the function has been
    # invoked. Comparing names catches new-function leaks without the
    # body churn.
    print -l ${(ko)functions} \
      | grep -vE "$_DUMP_HELPERS_RE"
    echo '=== VARIABLES ==='
    print -l ${(ko)parameters} \
      | grep -vE "$_ZSH_INTERNAL_VAR_RE" \
      | grep -vE "$_TEST_HARNESS_NOISE_RE"
  } > "$out"
}

_flox_dump "$1"

eval "$($FLOX_BIN activate -d $PROJECT_DIR)"
# `--print-script` requires the invocation type; `activate` exports
# `_FLOX_INVOCATION_TYPE` (here: in-place). See deactivate.bats.
eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"

_flox_dump "$2"
