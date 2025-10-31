_sed="@gnused@/bin/sed"

attach() {
  _flox_activation_state_dir="${1?}"
  shift
  _flox_invocation_type="${1?}"
  shift

  "$_flox_activate_tracer" "$_activate_d/attach.bash" START

  # Don't clobber STDERR or recommend 'exit' for non-interactive shells.
  # If inside a container, FLOX_ENV_DESCRIPTION won't be set, and we don't need to
  # print a message (although attach isn't reachable anyways)
  if [ "${_flox_invocation_type}" = "interactive" ] && [ -n "${FLOX_ENV_DESCRIPTION:-}" ]; then
    echo "✅ Attached to existing activation of environment '$FLOX_ENV_DESCRIPTION'" >&2
    echo "To stop using this environment, type 'exit'" >&2
    echo >&2
  fi

  # Replay the environment for the benefit of this shell.
  eval "$($_sed -e 's/^/unset /' -e 's/$/;/' "${_flox_activation_state_dir}/del.env")"
  eval "$($_sed -e 's/^/export /' -e 's/$/;/' "${_flox_activation_state_dir}/add.env")"

  "$_flox_activate_tracer" "$_activate_d/attach.bash" END
}
