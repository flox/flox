#!/usr/bin/env bash
# One rebuild bake with verbose logging + hi-res timestamps to break the
# 77s baseline into phases. We look for: populate (nix copy), the container
# layer write (streamLayeredImage), skopeo conversion, and image load.
set -uo pipefail
FLOX="${FLOX_BIN:-/Users/djsauble/Code/flox/target/debug/flox}"
ENVDIR="${TMPDIR:-/tmp}/svbench-env"
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_SANDBOX_OCI_AUTOBAKE=true
TS() { perl -MTime::HiRes=time -pe 'BEGIN{$|=1} printf "%.3f ", time()'; }
now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time()'; }

echo "== env change: install a package to force a rebuild =="
"$FLOX" install --dir "$ENVDIR" ripgrep >/dev/null 2>&1 || "$FLOX" install --dir "$ENVDIR" tree >/dev/null 2>&1

echo "== rebuild bake (-vv), timestamped =="
start=$(now)
"$FLOX" -vv activate --dir "$ENVDIR" --sandbox enforce --sandbox-backend oci -- true 2>&1 | TS
end=$(now)
echo "TOTAL_REBUILD=$(perl -e "printf '%.1f', $end-$start")s"
