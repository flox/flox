#!/usr/bin/env bash
#
# Usage: wait_for_services_status.sh [name:status ...]
#
# Waits for all of the name:status pairs to reach their desired status to
# prevent race conditions in our tests where we depend on something happening
# after a certain state.
#
set -euo pipefail

check_status() {
    name_status_pairs=("$@")
    status_output=$("$FLOX_BIN" services status --json)

    for status in "${name_status_pairs[@]}"; do
        service_name=$(echo $status | cut -d':' -f1)
        expected_status=$(echo $status | cut -d':' -f2)
        current_status=$(echo "$status_output" | jq --raw-output ".[] | select(.name==\"$service_name\") | .status")

        if [ "$current_status" != "$expected_status" ]; then
            echo "Service '$service_name' status current=$current_status, expected=$expected_status"
            return 1
        fi
    done

    echo "All services reached their expected status."
    return 0
}

export -f check_status
# Polling stops as soon as every service reports the expected status, so the
# budget only bounds how long a genuine failure takes to report. 1s was under
# what a healthy CI runner needs to move a service between states while it is
# also running the rest of the suite, which showed up as "status
# current=Stopped, expected=Completed" on runs where the service did reach the
# expected status a moment later.
timeout 10s bash -c "while ! check_status ${*}; do sleep 0.1; done"
