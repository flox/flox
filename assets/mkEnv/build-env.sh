#!/usr/bin/env sh

set -eu

NIX="$1";
SYSTEM="$2";
LOCKFILE_PATH="$3";
OUTLINK_PREFIX="$4";
ENV_FROM_LOCKFILE_PATH="$5";
"$NIX" build --file "$ENV_FROM_LOCKFILE_PATH" --out-link "$OUTLINK_PREFIX" --print-out-paths --arg lockfilePath "$LOCKFILE_PATH" --argstr system "$SYSTEM"
