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
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}" \
      "true" # is_in_place
    ;;
  *fish)
    echo "$_flox_activations attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$fish_pid --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    generate_fish_startup_commands \
      "$_flox_activate_tracelevel" \
      "$_FLOX_ACTIVATION_STATE_DIR" \
      "$_activate_d" \
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
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}"
    ;;
  # Any additions should probably be restored in zdotdir/* scripts
  *zsh)
    echo "$_flox_activations attach --runtime-dir \"$FLOX_RUNTIME_DIR\" --pid \$\$ --flox-env \"$FLOX_ENV\" --id \"$_FLOX_ACTIVATION_ID\" --remove-pid \"$expiring_pid\";"
    if [ -n "${ZDOTDIR:-}" ]; then
      echo "export FLOX_ORIG_ZDOTDIR=\"$ZDOTDIR\";"
    fi
    echo "export ZDOTDIR=\"$_zdotdir\";"

    FLOX_ZSH_INIT_SCRIPT="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
    generate_zsh_startup_script \
      "$_flox_activate_tracelevel" \
      "$_FLOX_ACTIVATION_STATE_DIR" \
      "$_activate_d" \
      "$FLOX_ENV" \
      "${_FLOX_ENV_CACHE:-}" \
      "${_FLOX_ENV_PROJECT:-}" \
      "${_FLOX_ENV_DESCRIPTION:-}" > "$FLOX_ZSH_INIT_SCRIPT"
    # self destruct
    if [ "$_flox_activate_tracelevel" -lt 2 ]; then
      echo "@coreutils@/bin/rm '$FLOX_ZSH_INIT_SCRIPT'" >> "$FLOX_ZSH_INIT_SCRIPT"
    fi

    # TODO: I don't think we should export this but it's needed by set-prompt.zsh
    echo "export _flox_activate_tracer=\"$_flox_activate_tracer\";"
    echo "source '$FLOX_ZSH_INIT_SCRIPT';"
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac

"$_flox_activate_tracer" "$_activate_d/attach-inplace.bash" END
