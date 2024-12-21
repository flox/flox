#!/bin/bash
set -o pipefail
path0=$(echo "$PATH" | cut -d: -f1)
if realpath "$path0" | grep -q "^/nix/store/"; then
  echo "FLOX_ACTIVATE_TRACE:" "$*" ✅ PATH 1>&2
else
  echo "FLOX_ACTIVATE_TRACE:" "$*" "❌ path[0] = $path0" 1>&2
fi
