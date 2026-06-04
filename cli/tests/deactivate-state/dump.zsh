#!/usr/bin/env zsh
# zsh state-dump helper for deactivate-state.bats.
#
# Snapshots zsh shell state to the path passed as $1. Designed to be
# invoked twice (pre-activate, post-deactivate) from a single zsh
# process so the diff catches anything activate/deactivate failed to
# restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR
#   _ALLOWED_LEAKS_RE, _TEST_HARNESS_NOISE_RE
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
_ZSH_INTERNAL_VAR_RE='^(_|0|argv|argv0|status|pipestatus|EGID|EUID|GID|UID|LINENO|RANDOM|SECONDS|EPOCHSECONDS|EPOCHREALTIME|funcfiletrace|funcsourcetrace|funcstack|functrace|HISTCMD|COLUMNS|LINES|f|out|_ZSH_INTERNAL_VAR_RE|_ZSH_ALLOWED_LEAKS_RE|_DUMP_HELPERS_RE)$'

_DUMP_HELPERS_RE='^(_flox_dump)$'

# zsh-specific intentional leaks not covered by the cross-shell list.
#   nohashcmds / nohashdirs — gen_rc/zsh.rs:84 has deactivate unset
#                             these to the zsh default (`setopt
#                             hashcmds; setopt hashdirs`). If pre had
#                             them set off (compinit toggles
#                             nohashdirs), deactivate over-restores to
#                             default rather than pre-state. Filtering
#                             keeps the test focused on flox-controlled
#                             state and not on default-vs-pre setopt
#                             drift.
#   _comp_assocs / chpwd_functions / precmd_functions
#                           — compinit / zsh hook plumbing pulled in by
#                             activate.d/zsh's compinit call.
#   new_fpath / old_fpath / profile_script_dirs
#                           — activate.d/zsh declares these without
#                             `local`, so they leak into the user's
#                             shell. Tracked separately in DEV-86; fix
#                             belongs in activate.d/zsh.
#   MANPATH / manpath       — when MANPATH is set pre-activate (CI
#                             runners and most interactive shells do
#                             this), zsh deactivate fails to restore
#                             it: post has neither MANPATH nor zsh's
#                             lowercase `manpath` mirror. The bash
#                             test happens to start without MANPATH
#                             set, so it doesn't trip the same path.
#                             Tracked separately in DEV-86.
_ZSH_ALLOWED_LEAKS_RE='^(nohashcmds|nohashdirs|_comp_assocs|chpwd_functions|precmd_functions|new_fpath|old_fpath|profile_script_dirs|MANPATH|manpath)$'

_flox_dump() {
  local out=$1
  {
    echo '=== SETOPT ==='
    setopt | sort | grep -vE "$_ZSH_ALLOWED_LEAKS_RE"
    echo '=== FUNCTIONS ==='
    # Names only: zsh autoloads are lazy, so their bodies differ between
    # the "shadow" entry (`# undefined` + `builtin autoload -XUz`) and
    # the fully-resolved entry that exists after the function has been
    # invoked. Comparing names catches new-function leaks without the
    # body churn.
    print -l ${(ko)functions} \
      | grep -vE "$_DUMP_HELPERS_RE" \
      | grep -vE "$_ALLOWED_LEAKS_RE"
    echo '=== VARIABLES ==='
    print -l ${(ko)parameters} \
      | grep -vE "$_ZSH_INTERNAL_VAR_RE" \
      | grep -vE "$_ALLOWED_LEAKS_RE" \
      | grep -vE "$_ZSH_ALLOWED_LEAKS_RE" \
      | grep -vE "$_TEST_HARNESS_NOISE_RE"
  } > "$out"
}

_flox_dump "$1"

eval "$($FLOX_BIN activate -d $PROJECT_DIR)"
# `--print-script` requires the invocation type; `activate` exports
# `_FLOX_INVOCATION_TYPE` (here: in-place). See deactivate.bats.
eval "$($FLOX_BIN deactivate --print-script "$_FLOX_INVOCATION_TYPE")"

_flox_dump "$2"
