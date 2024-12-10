expiring_pid="$$"
# Put a 5 second timeout on the activation
"$_flox_activations" \
  --runtime-dir "$FLOX_RUNTIME_DIR" \
  attach \
  --pid "$expiring_pid" \
  --flox-env "$FLOX_ENV" \
  --id "$_FLOX_ACTIVATION_ID" \
  --timeout-ms 5000

# "in-place" mode: emit activation commands in correct shell dialect by echoing
# the contents of the shell-specific activation script.  N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
case "$_flox_shell" in
  *bash)
    echo "$_flox_activations --runtime-dir \"$FLOX_RUNTIME_DIR\" attach --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_bash_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}"
    ;;
  *fish)
    echo "$_flox_activations --runtime-dir \"$FLOX_RUNTIME_DIR\" attach --pid \$fish_pid --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_fish_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}"
    ;;
  *tcsh)
    echo "$_flox_activations --runtime-dir \"$FLOX_RUNTIME_DIR\" attach --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_tcsh_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}"
    ;;
  # Any additions should probably be restored in zdotdir/* scripts
  *zsh)
    echo "export _FLOX_RESTORE_PATH=\"$PATH\";"
    echo "export _FLOX_RESTORE_MANPATH=\"$MANPATH\";"
    echo "$_flox_activations --runtime-dir \"$FLOX_RUNTIME_DIR\" attach --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    echo "export _flox_activate_tracelevel=\"$_flox_activate_tracelevel\";"
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    if [ -n "${ZDOTDIR:-}" ]; then
      echo "export FLOX_ORIG_ZDOTDIR=\"$ZDOTDIR\";"
    fi
    echo "export ZDOTDIR=\"$_zdotdir\";"
    echo "export _FLOX_ACTIVATION_STATE_DIR=\"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "export FLOX_ZSH_INIT_SCRIPT=\"$_activate_d/zsh\";"
    echo "export _activate_d=\"$_activate_d\";"
    echo "source '$_activate_d/zsh';"
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac
