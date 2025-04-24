#!/usr/bin/env bash

set -euxo pipefail

timeout 2 bash -c 'set -x; while [ ! -e "$(pwd)/pidfile" ]; do sleep .1; done'

"$FLOX_BIN" services status
"$FLOX_BIN" services stop

timeout 2 bash -c 'set -x; while kill -0 "$(cat "$(pwd)/pidfile")"; do sleep .1; done'
