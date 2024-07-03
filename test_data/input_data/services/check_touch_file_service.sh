#!/usr/bin/env bash

set -eo pipefail

function cleanup() {
    echo "shutting down"
    "$PROCESS_COMPOSE_BIN" down -u "$SOCKET_FILE" || true
}

echo "activating"
eval $("$FLOX_BIN" activate)
CONFIG_FILE="$FLOX_ENV/service-config.yaml"
SOCKET_FILE="${_FLOX_SERVICES_SOCKET}"

echo "starting the service"
# https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
"$PROCESS_COMPOSE_BIN" up -f "$CONFIG_FILE" --tui=false --unix-socket "$SOCKET_FILE" 3>&- &
sleep 1 # wait for the service to run
trap cleanup EXIT

echo "looking for file"
[ -e hello.txt ]
echo "found it"
