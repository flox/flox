#!/usr/bin/env bash
# Baseline benchmark: env-change -> flox activate (full rebuild) for the OCI
# sandbox backend. Measures total wall time + per-phase breakdown
# ([1/3] fill cache, [2/3] write layers = flox containerize + streamLayeredImage
# + skopeo, [3/3] load image). The store-volume fast path aims to eliminate
# most of [2/3] and all of [3/3].
set -uo pipefail

FLOX="${FLOX_BIN:-/Users/djsauble/Code/flox/target/debug/flox}"
ENVDIR="${TMPDIR:-/tmp}/svbench-env"
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_SANDBOX_OCI_AUTOBAKE=true

# Hi-res timestamp prefixer for phase-boundary detection.
TS() { perl -MTime::HiRes=time -pe 'BEGIN{$|=1} printf "%.3f ", time()'; }
now() { perl -MTime::HiRes=time -e 'printf "%.3f\n", time()'; }
mark() { echo ">>>>> [$(now)] $*"; }

echo "FLOX=$FLOX"; "$FLOX" --version
echo "ENVDIR=$ENVDIR"

rm -rf "$ENVDIR"; mkdir -p "$ENVDIR"

mark "INIT env"
"$FLOX" init --dir "$ENVDIR"
cat >> "$ENVDIR/.flox/env/manifest.toml" <<'EOF'

[options.sandbox]
backend = "oci"
EOF

mark "INSTALL initial packages (curl git jq — a realistic closure)"
"$FLOX" install --dir "$ENVDIR" curl git jq

mark "COLD BAKE start (first bake: downloads + cross-compile)"
cold_start=$(now)
"$FLOX" activate --dir "$ENVDIR" --sandbox enforce --sandbox-backend oci -- true 2>&1 | TS
cold_end=$(now)
mark "COLD BAKE done (elapsed=$(perl -e "printf '%.1f', $cold_end-$cold_start")s)"

echo "=== images after cold bake ==="
container image ls 2>&1 | grep -i "svbench-env" || true

mark "ENV CHANGE: install hello (small closure delta)"
"$FLOX" install --dir "$ENVDIR" hello

mark "REBUILD BAKE start (THE BASELINE NUMBER: env change -> activate)"
rb_start=$(now)
"$FLOX" activate --dir "$ENVDIR" --sandbox enforce --sandbox-backend oci -- true 2>&1 | TS
rb_end=$(now)
mark "REBUILD BAKE done (elapsed=$(perl -e "printf '%.1f', $rb_end-$rb_start")s)"

echo "=== images after rebuild ==="
container image ls 2>&1 | grep -i "svbench-env" || true

mark "DONE"
