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
for i in {1..5}; do
  if "$FLOX_BIN" services status | grep "Completed"; then
    break
  fi
  sleep .1
done
if [ "$i" -eq 5 ]; then
  exit 1
fi

process_compose_pids_before="$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")"
"$FLOX_BIN" services start
process_compose_pids_after="$(process_compose_pids_called_with_arg "$(pwd)/.flox/run")"

[[ "$process_compose_pids_after" != *"$process_compose_pids_before"* ]]
