NOT_READY="SOCKET_NOT_READY"

poll_services_status() {
  local _jq="@jq@/bin/jq"
  local _process_compose="@process_compose@/bin/process-compose"
  local socket_file="$1"
  local output
  output=$($_process_compose process list -u "$socket_file" -o json 2>&1)
  # Save whatever the current `pipefail` behavior is so it can be restored
  local saved_options
  saved_options=$(set +o)
  # We don't want to exit on pipe failures here
  set +o pipefail
  local parsed_json
  parsed_json=$(echo "$output" | "$_jq" -r -c '.[0].status' 2> /dev/null)
  # Restore the previous shell settings
  eval "$saved_options"
  # `parsed_json` will be a null string if anything went wrong
  echo "${parsed_json:-${NOT_READY}}"
}

wait_for_services_socket() {
  local _sleep="@coreutils@/bin/sleep"
  local socket_file="$1"
  local status
  status=$(poll_services_status "$socket_file")
  while [ "$status" == "$NOT_READY" ]; do
    "$_sleep" 0.02
    status=$(poll_services_status "$socket_file")
  done
}

start_services_blocking() {
  local _bash="@bash@/bin/bash"
  local _cat="@coreutils@/bin/cat"
  local _date="@coreutils@/bin/date"
  local _jq="@jq@/bin/jq"
  local _process_compose="@process_compose@/bin/process-compose"
  local _setsid="@util_linux@/bin/setsid"
  local _timeout="@coreutils@/bin/timeout"
  local config_file="$1"
  shift
  local socket_file="$1"
  shift
  local log_dir="$1"
  local timestamp_ms
  timestamp_ms=$("$_date" "+%Y%m%d%H%M%S%6N")
  local log_file
  log_file="${log_dir}/services.${timestamp_ms}.log"
  # process-compose will vomit all over your log files unless you tell it otherwise
  local previous_no_color="${NO_COLOR:-}"
  export NO_COLOR=1
  # [sic] within scripts setsid needs to be called twice to work properly:
  # <https://stackoverflow.com/a/25398828>

  # flox services start [service...] needs to be able to start some but not all
  # services
  if [ -n "${_FLOX_SERVICES_TO_START:-}" ]; then
    readarray -t services_to_start < <(echo "$_FLOX_SERVICES_TO_START" | "$_jq" -r '.[]')
    COMPOSE_SHELL="$_bash" "$_setsid" "$_setsid" "$_process_compose" up "${services_to_start[@]}" -f "$config_file" -u "$socket_file" -L "$log_file" --tui=false > /dev/null 2>&1 &
  else
    COMPOSE_SHELL="$_bash" "$_setsid" "$_setsid" "$_process_compose" up -f "$config_file" -u "$socket_file" -L "$log_file" --tui=false > /dev/null 2>&1 &
  fi
  # Make these functions available in subshells so that `timeout` can call them
  export -f wait_for_services_socket poll_services_status
  local activation_timeout="${_FLOX_SERVICES_ACTIVATE_TIMEOUT:-10}"
  local blocking_command="wait_for_services_socket \"$socket_file\""

  echo " ----- Starting services at: $("@coreutils@/bin/date" +"%T.%N")" >&2

  if ! "$_timeout" "$activation_timeout" $_bash -c "$blocking_command"; then
    if [ ! -e "$log_file" ]; then
      # If something failed before process-compose could write to the log file,
      # don't tell a user to look at the log file
      echo "❌ Failed to start services" >&2
      exit 1
    else
      echo "❌ Failed to start services:" >&2
      "$_cat" "$log_file" >&2
      exit 1
    fi
  fi

  echo " Finished starting services at: $("$_coreutils/bin/date" +"%T.%N") ------" >&2

  # Unset the helper functions so that they aren't passed to the user shell/command
  unset wait_for_services_socket poll_services_status
  if [ -z "$previous_no_color" ]; then
    # It wasn't previously set
    unset NO_COLOR
  else
    export NO_COLOR="$previous_no_color"
  fi
}

config_file="$FLOX_ENV/service-config.yaml"
start_services_blocking "$config_file" "$_FLOX_SERVICES_SOCKET" "$_FLOX_ENV_LOG_DIR"
