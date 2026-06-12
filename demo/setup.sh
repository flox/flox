#!/usr/bin/env bash
# Sandboxed-activation prototype — demo setup. Creates EVERYTHING:
# the demo project (+git repo), the flox env with mediated tools,
# and the out-of-project fixture files. Run once; demo/cleanup.sh
# removes it all (including any grants the demo adds).
#
# Assumes (per the demo's prereqs):
#   - the locally built `flox` is already first in PATH
#   - FLOX_FEATURES_SANDBOX_ACTIVATE=true is already exported
#     (verified below; this script sets it for its own calls)
set -euo pipefail

DEMO_DIR="${DEMO_DIR:-/tmp/sandbox-demo}"

# --- preflight -------------------------------------------------------------
if ! command -v flox >/dev/null 2>&1; then
  echo "ERROR: no 'flox' on PATH. Put your locally built flox first in PATH." >&2
  exit 1
fi
echo "Using flox: $(command -v flox)"
if [ "${FLOX_FEATURES_SANDBOX_ACTIVATE:-}" != "true" ]; then
  echo "WARNING: FLOX_FEATURES_SANDBOX_ACTIVATE is not 'true' in this shell." >&2
  echo "         The walkthrough assumes it is exported before you start." >&2
fi
# This script needs the flag itself (flox sandbox + the smoke test are gated).
export FLOX_FEATURES_SANDBOX_ACTIVATE=true

# --- demo project (lives in /tmp on purpose; /tmp is sandbox-allowed) -------
# Braces required: macOS system bash 3.2 pulls a following multibyte
# character (the ellipsis) into the variable name under `set -u`.
echo "Creating demo environment at ${DEMO_DIR}…"
rm -rf "$DEMO_DIR"
mkdir -p "$DEMO_DIR"
cd "$DEMO_DIR"
flox init >/dev/null

echo "Installing mediated tools (bash, coreutils, curl, git)…"
# On macOS the sandbox only mediates Nix-store / env-provided binaries
# (system tools are SIP-protected and escape the loader). Installing the
# tools into the environment makes the demo deterministic.
flox install bash coreutils curl git >/dev/null

echo "Seeding a sample project…"
printf 'def greet():\n    return 1\n' > app.py
git init -q
git config user.email demo@flox.dev
git config user.name  "Demo"
git add -A && git commit -qm "initial project"

# --- out-of-project fixtures (MUST stay under $HOME) ------------------------
# libsandbox treats /tmp as a built-in always-allowed prefix, checked before
# the sensitive set (package-builder/sandbox.c: allow_dirs_init). If these
# lived under /tmp they would be silently allowed and the demo's blocked /
# (sensitive) / ask beats would vanish. $HOME is out of policy, which is the
# whole story: a secret OUTSIDE the project that the agent must not read.
echo "Creating two HARMLESS fake secrets (removed by demo/cleanup.sh)…"
mkdir -p "$HOME/demo-secrets" "$HOME/demo-data"
printf 'API_KEY=sk-demo-FAKE-do-not-use\n' > "$HOME/demo-secrets/.env"
printf 'order_id,amount\n1001,42\n'        > "$HOME/demo-data/fixtures.csv"

# --- personal dev convenience (hardcoded on purpose) -------------------------
# The dev build's binaries (flox, flox-activations, flox-watchdog,
# libsandbox) are read from target/debug INSIDE the sandboxed session, which
# would otherwise generate receipts/denials. This is djsauble's machine-
# specific path — edit it if your checkout lives elsewhere.
echo "Granting the dev build directory (personal convenience)…"
flox sandbox allow '/Users/djsauble/Code/flox/target/debug/**' >/dev/null

# --- smoke test --------------------------------------------------------------
echo "Smoke-testing a sandboxed activation…"
flox activate --sandbox warn -- true >/dev/null

cat <<EOF

Setup complete. To run the demo:

    cd $DEMO_DIR

then follow demo/SCRIPT.md. Afterwards: bash demo/cleanup.sh
EOF
