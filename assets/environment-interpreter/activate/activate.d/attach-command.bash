"$_flox_activate_tracer" "$_activate_d/attach-command.bash" START

# "command" mode(s): invoke the user's shell with args that:
#   a. defeat the shell's normal startup scripts
#   b. source the relevant activation script
#   c. invoke the command in one of "stdin" or "-c" modes
# "-c" command mode: pass both [2] arguments unaltered to shell invocation
case "$_flox_shell" in
  *bash)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" --noprofile --norc -c "$FLOX_CMD"
    else
      RCFILE="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_bash_startup_commands \
        "$_flox_activate_tracelevel" \
        "$_FLOX_ACTIVATION_STATE_DIR" \
        "$_activate_d" \
        "$FLOX_ENV" \
        "${_FLOX_ENV_CACHE:-}" \
        "${_FLOX_ENV_PROJECT:-}" \
        "${_FLOX_ENV_DESCRIPTION:-}" \
        "false" \
        > "$RCFILE"
      # self destruct
      echo "@coreutils@/bin/rm '$RCFILE'" >> "$RCFILE"
      if [ -t 1 ]; then
        exec "$_flox_shell" --noprofile --rcfile "$RCFILE" -c "$FLOX_CMD"
      else
        # The bash --rcfile option only works for interactive shells
        # so we need to cobble together our own means of sourcing our
        # startup script for non-interactive shells.
        exec "$_flox_shell" --noprofile --norc -s <<< "source '$RCFILE' && $FLOX_CMD"
      fi
    fi
    ;;
  *fish)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$FLOX_CMD"
    else
      RCFILE="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_fish_startup_commands \
        "$_flox_activate_tracelevel" \
        "$_FLOX_ACTIVATION_STATE_DIR" \
        "$_activate_d" \
        "$FLOX_ENV" \
        "${_FLOX_ENV_CACHE:-}" \
        "${_FLOX_ENV_PROJECT:-}" \
        "${_FLOX_ENV_DESCRIPTION:-}" \
        > "$RCFILE"
      # self destruct
      echo "@coreutils@/bin/rm '$RCFILE'" >> "$RCFILE"
      exec "$_flox_shell" --init-command "source '$RCFILE'" -c "$FLOX_CMD"
    fi
    ;;
  *tcsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$FLOX_CMD"
    else
      export FLOX_ORIG_HOME="$HOME"
      export HOME="$_tcsh_home"

      # The tcsh implementation will source our custom `~/.tcshrc`,
      # which eventually sources $FLOX_TCSH_INIT_SCRIPT after the normal initialization.
      FLOX_TCSH_INIT_SCRIPT="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_tcsh_startup_commands \
        "$_flox_activate_tracelevel" \
        "$_FLOX_ACTIVATION_STATE_DIR" \
        "$_activate_d" \
        "${FLOX_ENV}" \
        "${_FLOX_ENV_CACHE:-}" \
        "${_FLOX_ENV_PROJECT:-}" \
        "${_FLOX_ENV_DESCRIPTION:-}" \
        > "$FLOX_TCSH_INIT_SCRIPT"
      # self destruct
      echo "@coreutils@/bin/rm '$FLOX_TCSH_INIT_SCRIPT'" >> "$FLOX_TCSH_INIT_SCRIPT"
      export FLOX_TCSH_INIT_SCRIPT

      exec "$_flox_shell" -m -c "$FLOX_CMD"
    fi
    ;;
  *zsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -o NO_GLOBAL_RCS -o NO_RCS -c "$FLOX_CMD"
    else
      if [ -n "${ZDOTDIR:-}" ]; then
        export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
      fi
      export ZDOTDIR="$_zdotdir"
      export FLOX_ZSH_INIT_SCRIPT="$_activate_d/zsh"
      # The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
      # automatically sourcing /etc/zshrc et al.
      exec "$_flox_shell" -o NO_GLOBAL_RCS -c "$FLOX_CMD"
    fi
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac

"$_flox_activate_tracer" "$_activate_d/attach-command.bash" END
