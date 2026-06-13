#!/usr/bin/env bash
# Claude Code PostToolUse hook: format a file after Edit/Write
# Usage: bash format.sh <formatter> [args...]
# Requires jq and the formatter to be on PATH (e.g. via nix develop).
# Exits 0 if dependencies are missing so we don't block development outside nix
# develop.

set -euo pipefail

FORMATTER=("$@")

if ! command -v jq &>/dev/null; then
  echo "jq not found, skipping - likely not inside nix develop" >&2
  exit 1
fi

if ! command -v "${FORMATTER[0]}" &>/dev/null; then
  echo "${FORMATTER[0]} not found, skipping - likely not inside nix develop" >&2
  exit 1
fi

# Extract file path from stdin JSON
FILE=$(jq -r '.tool_input.file_path')

"${FORMATTER[@]}" "$FILE"
