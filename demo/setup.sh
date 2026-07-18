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
# Sandbox backend the manifest declares: "oci" (default; Apple Container /
# podman), "openshell" (NVIDIA OpenShell — see demo/OPENSHELL.md),
# "modal" (Modal Sandboxes, cloud-remote — see demo/MODAL.md), or
# "docker-sbx" (Docker Sandboxes local microVM — see demo/DOCKER-SBX.md).
BACKEND="${BACKEND:-oci}"
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

case "$BACKEND" in
  oci) ;;
  openshell)
    # The openshell backend shells out to the OpenShell CLI and loads
    # images through Docker — check both up front so the first
    # activation doesn't fail mid-demo.
    if ! command -v openshell >/dev/null 2>&1; then
      echo "WARNING: 'openshell' not found on PATH." >&2
      echo "         Install: https://github.com/NVIDIA/OpenShell#install" >&2
    elif ! openshell status >/dev/null 2>&1; then
      echo "WARNING: the OpenShell gateway is not reachable" >&2
      echo "         ('openshell status' failed). Start/select it first." >&2
    fi
    if ! command -v docker >/dev/null 2>&1; then
      echo "WARNING: 'docker' not found on PATH (required by openshell)." >&2
    fi
    ;;
  modal)
    # The modal backend is cloud-remote: it bakes an image locally (Docker),
    # then generates a Modal launch program. Preflight distinguishes
    # CLI-missing from CLI-present-but-unauthenticated. The remote launch
    # needs a Modal account + a registry Modal can pull from; neither is set
    # up by this script (see demo/MODAL.md beats 2+).
    if ! command -v modal >/dev/null 2>&1; then
      echo "WARNING: 'modal' not found on PATH." >&2
      echo "         Install: flox install python313Packages.modal (or pip install modal)" >&2
    elif ! modal token info >/dev/null 2>&1; then
      echo "NOTE: the Modal CLI is present but not authenticated." >&2
      echo "      That is expected for the local beats; the remote launch" >&2
      echo "      needs 'modal token new' (see demo/MODAL.md beat 0)." >&2
    fi
    if ! command -v docker >/dev/null 2>&1; then
      echo "WARNING: 'docker' not found on PATH (required to bake the image)." >&2
    fi
    if [ -z "${FLOX_SANDBOX_MODAL_REGISTRY:-}" ]; then
      echo "NOTE: FLOX_SANDBOX_MODAL_REGISTRY is unset; the generated launcher" >&2
      echo "      will use a bare image tag. Set it to your registry prefix" >&2
      echo "      (e.g. docker.io/<user>) before the remote launch." >&2
    fi
    ;;
  docker-sbx)
    # The docker-sbx backend bakes an image locally (Docker), compiles the
    # manifest network policy into an `sbx` kit, and stops at the launch
    # boundary. Preflight distinguishes sbx-missing / daemon-down / too-old.
    # The microVM launch needs the `sbx` CLI (signed in) and a base image
    # adapted to sbx's kit contract; neither is set up by this script (see
    # demo/DOCKER-SBX.md).
    if ! command -v sbx >/dev/null 2>&1; then
      echo "WARNING: 'sbx' not found on PATH." >&2
      echo "         Install: flox install docker-sbx (or brew install docker/tap/sbx)" >&2
    fi
    if ! command -v docker >/dev/null 2>&1; then
      echo "WARNING: 'docker' not found on PATH (required to bake the image)." >&2
    elif ! docker info >/dev/null 2>&1; then
      echo "WARNING: the Docker daemon is not reachable ('docker info' failed)." >&2
      echo "         Start Docker Desktop or the Docker service before baking." >&2
    fi
    ;;
  *)
    echo "ERROR: BACKEND='$BACKEND' is not a demo backend (oci|openshell|modal|docker-sbx)." >&2
    exit 1
    ;;
esac

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
echo "Installing agent tooling (git, curl, which, python3)..."
"$FLOX_BIN" install git curl which python3 >/dev/null

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
python3 - "$DEMO_DIR/.flox/env/manifest.toml" "$BACKEND" <<'EOF'
import sys
path = sys.argv[1]
backend = sys.argv[2]
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
  # Pre-seeded agent auth: a gitignored .env at the project root is
  # the one host-writable, guest-readable channel (the project is the
  # only mount). Drop CLAUDE_CODE_OAUTH_TOKEN=... there (from
  # `claude setup-token` on the host) and the agent needs no
  # interactive login inside the sandbox — the OAuth URL it prints
  # cannot be copied out of a sandboxed session.
  if [ -f "$_proj/.env" ]; then
    set -a
    . "$_proj/.env"
    set +a
  fi
\'\'\'
'''
services = '''[services]
auto-start = true

[services.web]
command = "python3 -m http.server 8080"
'''
sandbox = f'''[options.sandbox]
backend = "{backend}"
'''
if backend in ("openshell", "modal", "docker-sbx"):
    # Grant the coding agent its API endpoints. On openshell the grant is
    # scoped to the exact claude binary (`binary` resolves to the locked
    # store path via the lockfile) and enforced at L7. On modal the host of
    # each :443 endpoint compiles into the native domain allowlist
    # (TLS/443-only); on docker-sbx each :443 host compiles into the sbx
    # kit's `network.allowedDomains` (HTTP/HTTPS domains). In both cloud/
    # microVM cases the binary/access/protocol scoping is recorded but not
    # enforceable, a declared lossiness. Either way, everything else stays
    # deny-by-default.
    sandbox += '''
[[options.sandbox.network]]
endpoint = "api.anthropic.com:443"
binary = "claude-code/.claude-wrapped"

[[options.sandbox.network]]
endpoint = "statsig.anthropic.com:443"
binary = "claude-code/.claude-wrapped"
'''
text = text.replace("[install]\n", install, 1)
text = text.replace("[hook]\n", hook, 1)
# Replace the empty [services] stub with an auto-starting web service.
text = text.replace("[services]\n", services, 1)
# Declare the sandbox backend (and, for openshell, the agent's network
# grants) so `cd` auto-activates into the guest with no live manifest
# edit — appended so [options.sandbox] is its own table.
text = text.rstrip() + "\n\n" + sandbox
with open(path, "w") as f:
    f.write(text)
EOF
"$FLOX_BIN" edit -f "$DEMO_DIR/.flox/env/manifest.toml" >/dev/null

echo "Seeding a sample project..."
printf 'def greet():\n    return 1\n' > app.py
# A tiny index.html so the auto-started web service serves a clean page
# (http.server serves index.html for '/' instead of a slow directory
# listing of the project, which includes the heavy .flox/ tree).
printf '<!doctype html><title>sandbox-demo</title>\n<h1>Hello from inside the flox sandbox</h1>\n' > index.html
# Agent credentials live under .claude/ and .env inside the project
# mount — never commit them.
printf '.claude/\n.env\n' > .gitignore
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
    export FLOX_VERSION=\`flox --version\`  # route the bake to this branch's builder
    eval "\$(flox hook-env --shell bash --shell-pid \$\$)"  # skip if already in your RC

Then:

    cd $DEMO_DIR

and follow demo/SCRIPT.md (backend "oci"), demo/OPENSHELL.md
(backend "openshell"), demo/MODAL.md (backend "modal"), or
demo/DOCKER-SBX.md (backend "docker-sbx").
Afterwards: bash demo/cleanup.sh

NOTE: the manifest already declares [options.sandbox]
backend = "$BACKEND", an auto-starting web service, and (openshell
and modal) [[options.sandbox.network]] grants for the agent's
Anthropic endpoints, so the first 'cd' auto-activates straight into
the sandbox. The first-ever bake
takes ~5-15 min (the builder VM compiles the pinned flox rev; later
bakes reuse its cache, ~2-5 min); to pre-bake off-camera, run:

    FLOX_SANDBOX_OCI_AUTOBAKE=true flox activate -- true

NOTE (modal): the modal backend is cloud-remote. It bakes the image
locally and generates a launch program, but the remote launch needs
a Modal account (modal token new) and a registry Modal can pull from
(export FLOX_SANDBOX_MODAL_REGISTRY=<prefix>). See demo/MODAL.md.
EOF
