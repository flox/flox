#!/usr/bin/env bash
# Sandboxed-activation prototype — demo setup. Creates EVERYTHING:
# the demo project (+git repo), the flox env with mediated tools,
# and the out-of-project fixture files. Run once; demo/cleanup.sh
# removes it all.
#
# Prerequisites (run from inside `nix develop` or equivalent dev shell):
#   - FLOX_FEATURES_SANDBOX_ACTIVATE=true is already exported
#   - FLOX_FEATURES_AUTO_ACTIVATE=true is already exported
#   - FLOX_BIN resolves to the locally built flox (set by dev shell)
#
# After setup the epilogue tells you the two shell setup steps the
# demo needs: alias flox to $FLOX_BIN and export the feature flags.
set -euo pipefail

DEMO_DIR="${DEMO_DIR:-$HOME/sandbox-demo}"
FLOX_BIN="${FLOX_BIN:-$(command -v flox)}"

# --- preflight -------------------------------------------------------------
if ! command -v "$FLOX_BIN" >/dev/null 2>&1; then
  echo "ERROR: FLOX_BIN='$FLOX_BIN' not found. Run from inside nix develop." >&2
  exit 1
fi
echo "Using flox: $FLOX_BIN"
if [ "${FLOX_FEATURES_SANDBOX_ACTIVATE:-}" != "true" ]; then
  echo "WARNING: FLOX_FEATURES_SANDBOX_ACTIVATE is not 'true' in this shell." >&2
  echo "         The walkthrough assumes it is exported before you start." >&2
fi
if [ "${FLOX_FEATURES_AUTO_ACTIVATE:-}" != "true" ]; then
  echo "WARNING: FLOX_FEATURES_AUTO_ACTIVATE is not 'true' in this shell." >&2
  echo "         The walkthrough assumes it is exported before you start." >&2
fi
export FLOX_FEATURES_SANDBOX_ACTIVATE=true
export FLOX_FEATURES_AUTO_ACTIVATE=true

# --- demo project ----------------------------------------------------------
echo "Creating demo environment at ${DEMO_DIR}..."
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"
"$FLOX_BIN" init >/dev/null

echo "Installing mediated tools (bash, coreutils, curl, git)..."
# Installing the tools into the environment makes demo beats deterministic.
# On macOS the sandbox only mediates Nix-store / env-provided binaries;
# system tools are SIP-protected and escape the loader. The installed tools
# are what the sandbox sees and confines.
"$FLOX_BIN" install bash coreutils curl git >/dev/null

echo "Seeding a sample project..."
printf 'def greet():\n    return 1\n' > app.py
git init -q
git config user.email demo@flox.dev
git config user.name  "Demo"
git add -A && git commit -qm "initial project"

# Pre-allow auto-activation so the first 'cd' triggers the sandbox
# consent prompt (not the generic auto-activate prompt).
echo "Pre-allowing auto-activation for ${DEMO_DIR}..."
"$FLOX_BIN" activate allow --dir "$DEMO_DIR"

# --- out-of-project fixtures -----------------------------------------------
# $HOME is outside the project, making it outside the default policy —
# that is the whole story: a secret the agent must not read.
echo "Creating two HARMLESS fake secrets (removed by demo/cleanup.sh)..."
mkdir -p "$HOME/demo-secrets" "$HOME/demo-data"
printf 'API_KEY=sk-demo-FAKE-do-not-use\n' > "$HOME/demo-secrets/.env"
printf 'order_id,amount\n1001,42\n'        > "$HOME/demo-data/fixtures.csv"

cat <<EOF

Setup complete.

Before running the demo, in your presentation shell:

    alias flox='$FLOX_BIN'
    export FLOX_FEATURES_SANDBOX_ACTIVATE=true
    export FLOX_FEATURES_AUTO_ACTIVATE=true

Then:

    cd $DEMO_DIR

and follow demo/SCRIPT.md. Afterwards: bash demo/cleanup.sh
EOF
