#!/usr/bin/env bash
# After benchmark: env-change -> flox activate with the store-volume refresh
# fast path (FLOX_SANDBOX_OCI_STORE_VOLUME=1). Compares against the 76s
# full-rebuild baseline.
#
# With the valve on, the FIRST activate is a refresh (marker miss), not a bake.
# For this (unpublished) rev, refresh #1 also builds flox-for-linux in the
# builder — a one-time per-rev cost. Refresh #2 / #3 are the steady-state warm
# numbers that matter.
#
# AUTOBAKE is intentionally OFF: if the refresh path fails it falls back to the
# bake path, which then fails fast (no tty) — surfacing the failure loudly
# instead of masking it behind a slow bake.
#
# Preconditions:
#   FLOX_BIN -> freshly built flox from this worktree (has the refresh code)
#   FLOX_REV -> the pushed git rev of this branch (builder fetches the flake)
set -uo pipefail

FLOX="${FLOX_BIN:-/Users/djsauble/Code/flox/_worktrees/store-volume-refresh/target/debug/flox}"
REV="${FLOX_REV:?set FLOX_REV to the pushed branch rev}"
ENVDIR="${TMPDIR:-/tmp}/svbench-after-env"

export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_SANDBOX_OCI_STORE_VOLUME=1
export _FLOX_CONTAINERIZE_FLAKE_REF_OR_REV="$REV"
unset FLOX_SANDBOX_OCI_AUTOBAKE

TS() { perl -MTime::HiRes=time -pe 'BEGIN{$|=1} printf "%.3f ", time()'; }
now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time()'; }
mark() { echo ">>>>> [$(now)] $*"; }
activate() { # $1=label
  local label="$1"; local s e
  s=$(now)
  "$FLOX" activate --dir "$ENVDIR" --sandbox enforce --sandbox-backend oci -- uname -sm 2>&1 | TS
  e=$(now)
  echo ">>>>> $label elapsed=$(perl -e "printf '%.1f', $e-$s")s"
}

echo "FLOX=$FLOX  REV=$REV"; "$FLOX" --version
rm -rf "$ENVDIR"; mkdir -p "$ENVDIR"

mark "INIT + initial closure (curl git jq)"
"$FLOX" init --dir "$ENVDIR" >/dev/null
cat >> "$ENVDIR/.flox/env/manifest.toml" <<'EOF'

[options.sandbox]
backend = "oci"
EOF
"$FLOX" install --dir "$ENVDIR" curl git jq >/dev/null

mark "REFRESH #1 (cold: builds flox-linux for this rev + env + ctx; one-time)"
activate REFRESH_1_COLD

echo "=== marker + binary cache after refresh #1 ==="
cat "$ENVDIR/.flox/cache/store-volume-refresh.json" 2>/dev/null && echo || echo "(no marker!)"
ls -la ~/.cache/flox/store-volume/ 2>/dev/null || echo "(no binary cache dir)"

mark "ENV CHANGE 1: install hello"
"$FLOX" install --dir "$ENVDIR" hello >/dev/null
mark "REFRESH #2 (warm: cached flox-bin direct-exec — THE number)"
activate REFRESH_2_WARM

mark "ENV CHANGE 2: install tree"
"$FLOX" install --dir "$ENVDIR" tree >/dev/null
mark "REFRESH #3 (warm: confirm)"
activate REFRESH_3_WARM

mark "DONE — baseline full rebuild was ~76s"
