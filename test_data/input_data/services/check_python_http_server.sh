#!/usr/bin/env bash

set -eo pipefail

eval $("$FLOX_BIN" activate)

# Start the server
"$PROCESS_COMPOSE_BIN" up -f "$CONFIG_FILE" --tui=false -u "$SOCKET_FILE" >/dev/null 2>&1 &

# there's a race condition here, the socket file may not exist until the server is up
sleep 1

# Check if everything is up and running
# echo "checking socket" >&3
[ -a "$SOCKET_FILE" ]
# echo "checking server" >&3
curl -I -s localhost:7890 | grep "HTTP/1.0 200 OK"
# echo "checking process-compose" >&3
status_output=$("$PROCESS_COMPOSE_BIN" process list -o json -u "$SOCKET_FILE")
status=$(echo "$status_output" | jq -r -c '.[0].status')
pid=$(echo "$status_output" | jq -r -c '.[0].pid')
[ "$status" == "Running" ]
# echo "shutting down" >&3
"$PROCESS_COMPOSE_BIN" down -u "$SOCKET_FILE"
