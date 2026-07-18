#!/usr/bin/env bash
# Remove every demo artifact, including grants, receipts, and
# any OCI images baked for the sandbox-demo environment.
set -uo pipefail
DEMO_DIR="${DEMO_DIR:-$HOME/sandbox-demo}"
FLOX_BIN="${FLOX_BIN:-$(command -v flox)}"

# Removing the env dir also removes its grants and provenance journal —
# they live at $DEMO_DIR/.flox/cache/sandbox/{grants.toml,journal.ndjson}.
# The sandbox consent prompt has no persistent state of its own (it
# is answered per-session via the hook).

# setup.sh pre-allowed auto-activation, which wrote an
# auto_activate_environments entry to the GLOBAL flox config —
# remove it before deleting the directory it points at.
"$FLOX_BIN" activate deny --dir "$DEMO_DIR" 2>/dev/null || true

rm -rf "$DEMO_DIR"
rm -rf "$HOME/demo-secrets" "$HOME/demo-data"
rm -f  "$HOME/sbx-pwned.txt"   # only exists if a warn-mode experiment wrote it

# Remove any OCI images baked for the sandbox-demo environment.
# Images are tagged <env-name>:<lockfile-hash12> plus a `latest` alias.
# List and remove all tags that match the environment name.
if command -v container >/dev/null 2>&1; then
  container image list --json 2>/dev/null | \
    python3 -c "
import json, sys, subprocess
images = json.load(sys.stdin)
for img in images:
  for tag in img.get('Tags', []):
    if tag.startswith('sandbox-demo:'):
      subprocess.run(['container', 'image', 'delete', tag],
                     check=False, capture_output=True)
      print(f'Removed OCI image: {tag}')
" 2>/dev/null || true
fi

# Remove openshell-backend leftovers: any lingering demo sandboxes
# (normally deleted by --no-keep on exit) and the Docker-side images
# baked under the -openshell suffixed repository.
if command -v openshell >/dev/null 2>&1; then
  openshell sandbox list -o json 2>/dev/null | \
    python3 -c "
import json, sys, subprocess
try:
  sandboxes = json.load(sys.stdin)
except Exception:
  sandboxes = []
for sb in sandboxes:
  name = sb.get('name', '')
  if name.startswith('flox-sandbox-demo-'):
    subprocess.run(['openshell', 'sandbox', 'delete', name],
                   check=False, capture_output=True)
    print(f'Removed OpenShell sandbox: {name}')
" 2>/dev/null || true
fi
# The docker-sbx backend launches local microVMs via the `sbx` CLI. Any
# lingering demo sandbox (normally removed on `sbx rm`) is cleaned up here.
if command -v sbx >/dev/null 2>&1; then
  sbx ls 2>/dev/null | awk 'NR>1 {print $1}' | \
    grep -E '^flox-sandbox-demo' | \
    while read -r name; do
      sbx rm --force "$name" >/dev/null 2>&1 && echo "Removed sbx sandbox: $name"
    done || true
fi

if command -v docker >/dev/null 2>&1; then
  # The openshell and modal backends both bake under the -openshell repo;
  # the modal backend additionally names its pushed registry image under the
  # -modal repo, which may have been retagged locally before a push; the
  # docker-sbx backend bakes under the -docker-sbx repo; the ona backend
  # bakes under the -ona repo.
  docker image ls --format '{{.Repository}}:{{.Tag}}' 2>/dev/null | \
    grep -E '^sandbox-demo-(openshell|modal|docker-sbx|ona):' | \
    while read -r tag; do
      docker rmi "$tag" >/dev/null 2>&1 && echo "Removed Docker image: $tag"
    done || true
fi

# The ona backend writes a committed devcontainer hand-off at the repo root
# (.devcontainer/devcontainer.json), not under .flox/cache. It is removed with
# $DEMO_DIR above, but a demo run may have left a copy in the current project
# if $DEMO_DIR was overridden — surface it rather than silently orphaning it.
if [ -f "$DEMO_DIR/.devcontainer/devcontainer.json" ]; then
  rm -rf "$DEMO_DIR/.devcontainer"
  echo "Removed ona devcontainer hand-off under $DEMO_DIR/.devcontainer."
fi

# demo/host-env leftovers: the gateway-register service writes persistent
# CLI state under ~/.config/openshell (and may have switched the active
# gateway selection). The gateway's own TLS/config live in the env cache
# and are removed with it.
if [ -d "$HOME/.config/openshell/gateways/flox-demo" ]; then
  rm -rf "$HOME/.config/openshell/gateways/flox-demo"
  echo "Removed host-env gateway registration 'flox-demo'."
  echo "If it was your active gateway, re-select the previous one:"
  echo "  openshell gateway select <name>"
fi
rm -rf "$(dirname "$0")/openshell-setup/.flox/cache/openshell" 2>/dev/null || true

echo "Demo artifacts removed (env, grants, journal, fixtures, images)."
