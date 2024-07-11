#!/usr/bin/env bash

set -euo pipefail

export PC_SOCKET_PATH="${_FLOX_SERVICES_SOCKET}"
SERVICE_CONFIG="${FLOX_ENV}/service-config.yaml"

function cleanup() {
    echo "Shutting down process-compose"
    # Need to allow for errors in the shutdown process:
    # https://github.com/F1bonacc1/process-compose/issues/197
    "$PROCESS_COMPOSE_BIN" down || true
}

# TODO: Replace when we have `flox activate --start-services`.
echo "Starting process-compose"
# https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
"$PROCESS_COMPOSE_BIN" up --config "$SERVICE_CONFIG" --tui=false --keep-tui 3>&- &

# TODO: Replace when exiting the activation stops `process-compose`.
trap cleanup EXIT

# TODO: Replace when `flox activate --start-services` waits for socket.
echo "Waiting for process-compose socket"
timeout 2s bash -c "while [ ! -e \"$PC_SOCKET_PATH\" ]; do sleep 0.1; done"
