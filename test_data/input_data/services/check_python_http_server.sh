#!/usr/bin/env bash

set -eo pipefail

function cleanup() {
    echo "shutting down"
    "$PROCESS_COMPOSE_BIN" down -u "$SOCKET_FILE"
}

echo "activating"
eval $("$FLOX_BIN" activate)
CONFIG_FILE="$FLOX_ENV/service-config.yaml"
SOCKET_FILE="$PWD/service.sock"

echo "checking python"
which python3
python3 --version

echo "work_dir: $PWD"
echo "socket_file: $SOCKET_FILE"
echo "config_file: $CONFIG_FILE"

# Start the server
# https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
"$PROCESS_COMPOSE_BIN" up -f "$CONFIG_FILE" --tui=false --use-uds --unix-socket "$SOCKET_FILE" 3>&- &
trap cleanup EXIT

echo "waiting for socket"
timeout 2s bash -c "while [ ! -e \"$SOCKET_FILE\" ]; do sleep 0.1; done"

# Check if everything is up and running
echo "checking socket"
[ -a "$SOCKET_FILE" ]
echo "checking server"
curl --head --fail --silent localhost:7890
echo "checking process-compose"
status_output=$("$PROCESS_COMPOSE_BIN" process list -o json -u "$SOCKET_FILE")
status=$(echo "$status_output" | jq -r -c '.[0].status')
pid=$(echo "$status_output" | jq -r -c '.[0].pid')
[ "$status" == "Running" ]
