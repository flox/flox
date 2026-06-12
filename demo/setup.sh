#!/usr/bin/env bash
# Sandboxed-activation prototype — demo setup.
#
# Run ONCE before recording, from inside the dev shell of the
# prototype worktree:
#
#   cd /path/to/flox/_worktrees/sandboxed-activation
#   nix develop                 # provides the prototype `flox` build
#   bash demo/setup.sh
#   source demo/setup.sh        # (or re-run the two exports it prints)
#
# It creates a throwaway environment with mediated (Nix-store)
# tools, a sample project, and two harmless FAKE secrets so the
# blocking demos never touch your real credentials.
set -euo pipefail

# The prototype binary is exported by the dev shell as FLOX_BIN.
FLOX="${FLOX_BIN:-$PWD/target/debug/flox}"
DEMO_DIR="${DEMO_DIR:-$HOME/sandbox-demo}"

echo "Building the prototype (just build)…"
just build >/dev/null

echo "Creating demo environment at $DEMO_DIR…"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"
"$FLOX" init >/dev/null

echo "Installing mediated tools (bash, coreutils, curl, git)…"
# On macOS the sandbox only mediates Nix-store / env-provided
# binaries (system tools are SIP-protected and escape). Installing
# the tools into the environment makes the demo deterministic.
"$FLOX" install bash coreutils curl git >/dev/null

echo "Seeding a sample project…"
printf 'def greet():\n    return 1\n' > app.py
git init -q
git config user.email demo@flox.dev
git config user.name  "Demo"
git add -A && git commit -qm "initial project" || true

echo "Creating two HARMLESS fake secrets (cleaned up by demo/cleanup.sh)…"
mkdir -p "$HOME/demo-secrets" "$HOME/demo-data"
printf 'API_KEY=sk-demo-FAKE-do-not-use\n' > "$HOME/demo-secrets/.env"
printf 'order_id,amount\n1001,42\n'        > "$HOME/demo-data/fixtures.csv"

cat <<EOF

Setup complete.

Before recording, in this same dev shell run:

    alias flox="$FLOX"
    export FLOX_FEATURES_SANDBOX_ACTIVATE=true
    cd "$DEMO_DIR"

Then follow demo/SCRIPT.md. Afterwards: bash demo/cleanup.sh
EOF
