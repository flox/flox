#!/usr/bin/env bash
# Sandboxed-activation prototype — demo setup. Creates EVERYTHING:
# the demo project (+git repo), the flox env with the agent tooling,
# and the out-of-project fixture files. Run once; demo/cleanup.sh
# removes it all.
#
# Prerequisites (run from inside `nix develop` or equivalent dev shell):
#   - FLOX_FEATURES_SANDBOX_ACTIVATE=true is already exported
#   - FLOX_FEATURES_AUTO_ACTIVATE=true is already exported
#   - FLOX_BIN resolves to the locally built flox (set by dev shell)
#
# After setup the epilogue tells you the shell setup steps the demo
# needs: alias flox to $FLOX_BIN and export the feature flags.
set -euo pipefail

DEMO_DIR="${DEMO_DIR:-$HOME/sandbox-demo}"
# `|| true` so an empty result reaches the friendly preflight below
# instead of aborting silently under `set -e`.
FLOX_BIN="${FLOX_BIN:-$(command -v flox || true)}"

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

# The environment is the agent's whole toolbox: the OCI image bakes
# the closure, so anything the agent needs at run time must be
# installed here. (The guest image always carries a baked-in bash +
# coreutils independent of the manifest — these installs are for the
# demo workload, not a sandbox requirement.)
echo "Installing agent tooling (git, curl, which)..."
"$FLOX_BIN" install git curl which >/dev/null

# Two manifest additions, applied in one edit:
#
# 1. flox/claude-code, in its own pkg-group so it resolves
#    independently of the base tools.
# 2. An agent-state hook: point Claude Code's config dir into the
#    project. The guest is ephemeral (--rm) and only the project
#    directory is mounted, so this is the one place agent state
#    (auth credentials, onboarding, permission-mode settings) can
#    survive between sandbox sessions. Seed
#    onboarding-complete and folder trust so the first in-guest run
#    reaches the login prompt with minimal ceremony.
echo "Adding claude-code and the agent-state hook to the manifest..."
python3 - "$DEMO_DIR/.flox/env/manifest.toml" <<'EOF'
import sys
path = sys.argv[1]
with open(path) as f:
    text = f.read()
install = '''[install]
claude-code.pkg-path = "flox/claude-code"
claude-code.pkg-group = "claude-code"
'''
hook = '''[hook]
on-activate = \'\'\'
  # In the OCI guest FLOX_ENV_PROJECT is unset, so the $PWD fallback
  # applies — it equals the project root because the demo always
  # activates from there (the container workdir is the project).
  _proj="${FLOX_ENV_PROJECT:-$PWD}"
  export CLAUDE_CONFIG_DIR="$_proj/.claude"
  mkdir -p "$CLAUDE_CONFIG_DIR"
  # Seed onboarding-complete and folder trust (best effort — a trust
  # prompt may still appear on first run; accepting it persists here).
  if [ ! -f "$CLAUDE_CONFIG_DIR/.claude.json" ]; then
    printf '{"hasCompletedOnboarding":true,"projects":{"%s":{"hasTrustDialogAccepted":true}}}' \
      "$_proj" > "$CLAUDE_CONFIG_DIR/.claude.json"
  fi
\'\'\'
'''
text = text.replace("[install]\n", install, 1)
text = text.replace("[hook]\n", hook, 1)
with open(path, "w") as f:
    f.write(text)
EOF
"$FLOX_BIN" edit -f "$DEMO_DIR/.flox/env/manifest.toml" >/dev/null

echo "Seeding a sample project..."
printf 'def greet():\n    return 1\n' > app.py
# Agent credentials live under .claude/ inside the project mount —
# never commit them.
printf '.claude/\n' > .gitignore
git init -q
git config user.email demo@flox.dev
git config user.name  "Demo"
git add -A && git commit -qm "initial project"

# Pre-allow auto-activation so the first 'cd' triggers the sandbox
# consent prompt (not the generic auto-activate prompt).
echo "Pre-allowing auto-activation for ${DEMO_DIR}..."
"$FLOX_BIN" activate allow --dir "$DEMO_DIR"

# --- out-of-project fixtures -----------------------------------------------
# $HOME is outside the project, so it simply does not exist inside the
# guest — that is the whole story: a secret the agent cannot even see.
echo "Creating a HARMLESS fake secret (removed by demo/cleanup.sh)..."
mkdir -p "$HOME/demo-secrets"
printf 'API_KEY=sk-demo-FAKE-do-not-use\n' > "$HOME/demo-secrets/.env"

cat <<EOF

Setup complete.

Before running the demo, in your presentation shell:

    alias flox='$FLOX_BIN'
    export FLOX_FEATURES_SANDBOX_ACTIVATE=true
    export FLOX_FEATURES_AUTO_ACTIVATE=true
    export GITHUB_TOKEN=ghp-demo-FAKE   # for the token-isolation beat

Then:

    cd $DEMO_DIR

and follow demo/SCRIPT.md. Afterwards: bash demo/cleanup.sh

NOTE: the first bake after setup takes ~2-5 min. To pre-bake
off-camera, add the two [options] lines from SCRIPT.md section 0 to
the manifest, then run:

    FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true
EOF
