#!/usr/bin/env fish
# fish state-dump helper for deactivate-state.bats.
#
# Snapshots fish shell state to the path passed as $argv[1]. Designed
# to be invoked twice (pre-activate, post-deactivate) from a single
# fish process so the diff catches anything activate/deactivate failed
# to restore.
#
# Required env (exported by the bats test):
#   FLOX_BIN, PROJECT_DIR
#   _ALLOWED_LEAKS_RE, _TEST_HARNESS_NOISE_RE
#
# Usage:
#   dump.fish <pre.txt> <post.txt>

set --export LC_ALL C
set --export FLOX_FEATURES_AUTO_ACTIVATE true
set --export FLOX_SHELL (command -v fish)

# fish-internal volatile names plus our own dump helpers, filtered
# from variable and function lists.
#   _flox_dump / out / f      — this script's locals
#   status / argv / pipestatus / _
#                             — fish auto-vars
#   fish_pid / fish_kill_signal / last_pid / SHLVL / PWD / OLDPWD
#                             — process state that may differ between
#                               the two snapshots
#   FISH_VERSION              — fish auto-var
set _FISH_INTERNAL_VAR_RE '^(_flox_dump|out|f|status|argv|pipestatus|_|fish_pid|fish_kill_signal|last_pid|SHLVL|PWD|OLDPWD|FISH_VERSION|history|CMD_DURATION|hostname)$'

set _DUMP_HELPERS_RE '^(_flox_dump)$'

function _flox_dump
    set out $argv[1]
    begin
        echo '=== FUNCTIONS ==='
        # Names only: fish lazy-loads functions like the vi key bindings,
        # so their bodies appear in post but not pre. Comparing names
        # catches new-function leaks without the body churn.
        functions --names \
          | string split ', ' \
          | sort \
          | grep -vE "$_DUMP_HELPERS_RE" \
          | grep -vE "$_ALLOWED_LEAKS_RE"
        echo '=== VARIABLES ==='
        set --names \
          | string split ' ' \
          | sort -u \
          | grep -vE "$_FISH_INTERNAL_VAR_RE" \
          | grep -vE "$_ALLOWED_LEAKS_RE" \
          | grep -vE "$_TEST_HARNESS_NOISE_RE"
    end > $out
end

_flox_dump $argv[1]

eval "$($FLOX_BIN activate -d $PROJECT_DIR)"
eval "$($FLOX_BIN deactivate --print-script)"

_flox_dump $argv[2]
