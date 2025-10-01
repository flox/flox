_sed="@gnused@/bin/sed"

attach() {
  _flox_activation_state_dir="${1?}"
  shift

  "$_flox_activate_tracer" "$_activate_d/attach.bash" START

  # If interactive and a command has not been passed, this is an interactive
  # activate,
  # and we print a message to the user
  # If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
  # print a message (although attach isn't reachable anyways)
  if [ -t 1 ] && [ $# -eq 0 ] && [ -n "${FLOX_ENV_DESCRIPTION:-}" ]; then
    echo "✅ Attached to existing activation of environment '$FLOX_ENV_DESCRIPTION'" >&2
    echo "To stop using this environment, type 'exit'" >&2
    echo >&2
  fi

  # Replay the environment for the benefit of this shell.
  eval "$($_sed -e 's/^/unset /' -e 's/$/;/' "${_flox_activation_state_dir}/del.env")"
  eval "$($_sed -e 's/^/export /' -e 's/$/;/' "${_flox_activation_state_dir}/add.env")"

  "$_flox_activate_tracer" "$_activate_d/attach.bash" END
}
