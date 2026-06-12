#!/usr/bin/env bash
# Remove every demo artifact, including grants and receipts.
set -uo pipefail
DEMO_DIR="${DEMO_DIR:-/tmp/sandbox-demo}"

# Removing the env dir also removes its grants and provenance journal —
# they live at $DEMO_DIR/.flox/cache/sandbox/{grants.toml,journal.ndjson}
# (per-environment, incl. the target/debug convenience grant). The ask
# pending queue is broker-memory only and dies with the session, so there
# is nothing else to clear.
rm -rf "$DEMO_DIR"
rm -rf "$HOME/demo-secrets" "$HOME/demo-data"
rm -f  "$HOME/sbx-pwned.txt"   # only exists if a warn-mode experiment wrote it
echo "Demo artifacts removed (env, grants, journal, fixtures)."
