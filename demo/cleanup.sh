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

echo "Demo artifacts removed (env, grants, journal, fixtures, OCI images)."
