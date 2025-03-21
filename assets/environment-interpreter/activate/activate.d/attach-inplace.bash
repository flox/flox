# shellcheck disable=SC2154
"$_flox_activate_tracer" "$_activate_d/attach-inplace.bash" START

expiring_pid="$$"
# Put a 5 second timeout on the activation
# shellcheck disable=SC2154
"$_flox_activations" \
  attach \
  --runtime-dir "$FLOX_RUNTIME_DIR" \
  --pid "$expiring_pid" \
  --flox-env "$FLOX_ENV" \
  --id "$_FLOX_ACTIVATION_ID" \
  --timeout-ms 5000

# "in-place" mode: emit activation commands in correct shell dialect by echoing
# the contents of the shell-specific activation script.  N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
# shellcheck disable=SC2154
case "$_flox_shell" in
  *bash)
    echo "$_flox_activations  attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_bash_startup_commands \
      "$_flox_activate_tracelevel" \
      "$_FLOX_ACTIVATION_STATE_DIR" \
      "$_activate_d" \
      "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" \
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}"
    ;;
  *fish)
    echo "$_flox_activations attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$fish_pid --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_fish_startup_commands \
      "$_flox_activate_tracelevel" \
      "$_FLOX_ACTIVATION_STATE_DIR" \
      "$_activate_d" \
      "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" \
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}"
    ;;
  *tcsh)
    echo "$_flox_activations attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_tcsh_startup_commands \
      "$_flox_activate_tracelevel" \
      "$_FLOX_ACTIVATION_STATE_DIR" \
      "$_activate_d" \
      "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" \
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}"
    ;;
  # Any additions should probably be restored in zdotdir/* scripts
  *zsh)
    echo "$_flox_activations attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    echo "export _flox_activate_tracelevel=\"$_flox_activate_tracelevel\";"
    # Propagate required variables that are documented as exposed.
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    # Propagate optional variables that are documented as exposed.
    for var_key in FLOX_ENV_CACHE FLOX_ENV_PROJECT FLOX_ENV_DESCRIPTION; do
      eval "var_val=\${_$var_key-}"
      if [ -n "$var_val" ]; then
        echo "export $var_key='$var_val';"
      else
        echo "unset $var_key;"
      fi
    done
    if [ -n "${ZDOTDIR:-}" ]; then
      echo "export FLOX_ORIG_ZDOTDIR=\"$ZDOTDIR\";"
    fi
    echo "export ZDOTDIR=\"$_zdotdir\";"
    echo "export _FLOX_ACTIVATION_STATE_DIR=\"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "export FLOX_ZSH_INIT_SCRIPT=\"$_activate_d/zsh\";"
    echo "export _activate_d=\"$_activate_d\";"
    echo "export _flox_activate_tracer=\"$_flox_activate_tracer\";"
    echo "source '$_activate_d/zsh';"
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac

"$_flox_activate_tracer" "$_activate_d/attach-inplace.bash" END
