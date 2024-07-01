#!/usr/bin/env bash

set -eo pipefail

function cleanup() {
    echo "shutting down" >&3
    "$PROCESS_COMPOSE_BIN" down -u "$SOCKET_FILE"
}

echo "activating" >&3
eval $("$FLOX_BIN" activate)
CONFIG_FILE="$FLOX_ENV/service-config.yaml"
SOCKET_FILE="$PWD/service.sock"

echo "checking python" >&3
which python3 >&3
python3 --version >&3

echo "work_dir: $PWD" >&3
echo "socket_file: $SOCKET_FILE" >&3
echo "config_file: $CONFIG_FILE" >&3

# Start the server
# https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
"$PROCESS_COMPOSE_BIN" up -f "$CONFIG_FILE" --tui=false -u "$SOCKET_FILE" 3>&- 2>&1 &
trap cleanup EXIT

echo "waiting for socket" >&3
timeout 2s bash -c "while [ ! -e \"$SOCKET_FILE\" ]; do sleep 0.1; done"

# Check if everything is up and running
echo "checking socket" >&3
[ -a "$SOCKET_FILE" ]
echo "checking server" >&3
curl_output=$(curl -I -s localhost:7890)
[ "$(echo "$curl_output" | head -n 1)" == "HTTP/1.0 200 OK" ]
echo "checking process-compose" >&3
status_output=$("$PROCESS_COMPOSE_BIN" process list -o json -u "$SOCKET_FILE")
status=$(echo "$status_output" | jq -r -c '.[0].status')
pid=$(echo "$status_output" | jq -r -c '.[0].pid')
[ "$status" == "Running" ]
