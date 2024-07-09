#!/usr/bin/env bash

set -eo pipefail

echo "activating"
eval "$("$FLOX_BIN" activate)"
CONFIG_FILE="$FLOX_ENV/service-config.yaml"
SOCKET_FILE="${_FLOX_SERVICES_SOCKET}"

echo "starting the service"
# https://bats-core.readthedocs.io/en/stable/writing-tests.html#file-descriptor-3-read-this-if-bats-hangs
timeout 2s "$PROCESS_COMPOSE_BIN" up -f "$CONFIG_FILE" --tui=false --unix-socket "$SOCKET_FILE" 3>&-

echo "looking for file"
[ -e hello.txt ]
echo "found it"
