#!/usr/bin/env bash

set -euo pipefail

export PC_SOCKET_PATH="${_FLOX_SERVICES_SOCKET}"

function cleanup() {
    echo "Shutting down process-compose" >&2
    # Need to allow for errors in the shutdown process:
    # https://github.com/F1bonacc1/process-compose/issues/197
    "$PROCESS_COMPOSE_BIN" down || true
}

# TODO: Replace when exiting the activation stops `process-compose`.
trap cleanup EXIT
