set -euo pipefail

# TODO: not very DRY, but I just copied this out of services.bats
process_compose_pids_called_with_arg() {
  # This is a hack to essentially do a `pgrep` without having access to `pgrep`.
  # The `ps` prints `<pid> <cmd>`, then we use two separate `grep`s so that the
  # grep command itself doesn't get listed when we search for the data dir.
  # The `cut` just extracts the PID.
  pattern="$1"
  ps_output="$(ps -eo pid,args)"
  process_composes="$(echo "$ps_output" | grep process-compose)"
  matches="$(echo "$process_composes" | grep "$pattern")"
  # This is a load-bearing 'xargs', it strips leading/trailing whitespace that
  # trips up 'cut'
  pids="$(echo "$matches" | xargs | cut -d' ' -f1)"
  echo "$pids"
}


# Make sure we avoid a race of service one failing to complete
"${TESTS_DIR}"/services/wait_for_service_status.sh one:Completed

process_compose_pids_before="$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")"
"$FLOX_BIN" services start

export process_compose_pids_before
export -f process_compose_pids_called_with_arg
timeout 2s bash -c '
  while [[ "$process_compose_pids_before" == "$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")" ]]; do
    sleep 0.1
  done
'
