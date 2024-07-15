#!/usr/bin/env bash

set -eo pipefail

echo "activating"
exec 3>&- # close the file descriptor
eval $("$FLOX_BIN" activate --start-services)
sleep 1 # wait for the services to start

echo "stopping process-compose"
"$PROCESS_COMPOSE_BIN" down -u "${_FLOX_SERVICES_SOCKET}" || true

echo "looking for file"
[ -e hello.txt ]
echo "found it"
