export _netcat="@netcat@"

function flox_portgrab() {
  # Function which allocates and immediately releases ephemeral TCP port,
  # used to identify available ports for use by services.
  #
  # Usage: port="$(portgrab)"
  #
  # - invokes nc as _server_ in background with args:
  #   -l: listen, rather than connect, mode
  #   -n: do not perform name resolution
  #   -v: be verbose (so we can identify bound port number)
  #   0.0.0.0: bind to all interfaces
  #   0: get an ephemeral port
  # - parses port number from expected output to stderr, e.g.:
  #     Listening on 0.0.0.0 41669
  # - invokes nc as _client_ to connect to server, which shuts it down
  local -a words
  ( $_netcat/bin/nc -lnv 0.0.0.0 0 & ) 2>&1 | while read -r -a words; do
    if [ ${#words[@]} -eq 4 ] && \
       [ "${words[0]}" = "Listening" ] && \
       [ "${words[1]}" = "on" ] && \
       [ "${words[2]}" = "0.0.0.0" ]; then
      local _port="${words[3]}"
      if $_netcat/bin/nc -N 0.0.0.0 "$_port" </dev/null >/dev/null 2>&1; then
        # Success! Echo port and return.
        echo "$_port"
        return 0
      else
        # Not sure what would cause this, but return to avoid looping further.
        return 1
      fi
    fi
  done
  # Return 1 to indicate failure.
  return 1
}
