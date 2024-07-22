#!/usr/bin/env bash

set -euo pipefail

export PC_SOCKET_PATH="${_FLOX_SERVICES_SOCKET}"

function cleanup() {
    echo "Shutting down process-compose"
    # Need to allow for errors in the shutdown process:
    # https://github.com/F1bonacc1/process-compose/issues/197
    "$PROCESS_COMPOSE_BIN" down || true
}

# TODO: Replace when exiting the activation stops `process-compose`.
trap cleanup EXIT

# TODO: Replace when `flox activate --start-services` waits.
echo "Waiting for process-compose to start"
timeout 2s bash -c '
    while ! process-compose process list -o wide >/dev/null 2>&1; do
        sleep 0.1
    done
'
