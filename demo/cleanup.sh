#!/usr/bin/env bash
# Remove the throwaway demo environment and the fake secrets.
set -uo pipefail
DEMO_DIR="${DEMO_DIR:-$HOME/sandbox-demo}"
rm -rf "$DEMO_DIR"
rm -rf "$HOME/demo-secrets" "$HOME/demo-data"
rm -f  "$HOME/sbx-pwned.txt"
echo "Demo artifacts removed."
