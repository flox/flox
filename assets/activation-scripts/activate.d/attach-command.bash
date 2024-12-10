# "command" mode(s): invoke the user's shell with args that:
#   a. defeat the shell's normal startup scripts
#   b. source the relevant activation script
#   c. invoke the command in one of "stdin" or "-c" modes
if [ -n "$FLOX_TURBO" ]; then
  # "turbo command" mode: simply invoke the provided command and args
  # from *this shell* without paying the cost of invoking the userShell.
  if [ -n "${FLOX_SET_ARG0:-}" ]; then
    # Wrapped binary from `flox build`.
    exec -a "$FLOX_SET_ARG0" "$@"
  else
    # We cannot exec here because we support bash shell internal commands.
    "$@"
    exit $?
  fi
fi

# Export PATH and MANPATH to restore in shell-specific activate scripts.
export _FLOX_RESTORE_PATH="$PATH"
export _FLOX_RESTORE_MANPATH="$MANPATH"

# "-c" command mode: pass both [2] arguments unaltered to shell invocation
case "$_flox_shell" in
  *bash)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" --noprofile --norc -c "$*"
    else
      RCFILE="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_bash_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" > "$RCFILE"
      # self destruct
      echo "@coreutils@/bin/rm '$RCFILE'" >> "$RCFILE"
      if [ -t 1 ]; then
        exec "$_flox_shell" --noprofile --rcfile "$RCFILE" -c "$*"
      else
        # The bash --rcfile option only works for interactive shells
        # so we need to cobble together our own means of sourcing our
        # startup script for non-interactive shells.
        exec "$_flox_shell" --noprofile --norc -s <<< "source '$RCFILE' && $*"
      fi
    fi
    ;;
  *fish)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$*"
    else
      RCFILE="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_fish_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" > "$RCFILE"
      # self destruct
      echo "@coreutils@/bin/rm '$RCFILE'" >> "$RCFILE"
      exec "$_flox_shell" --init-command "source '$RCFILE'" -c "$*"
    fi
    ;;
  *tcsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$*"
    else
      export FLOX_ORIG_HOME="$HOME"
      export HOME="$_tcsh_home"

      # The tcsh implementation will source our custom `~/.tcshrc`,
      # which eventually sources $FLOX_TCSH_INIT_SCRIPT after the normal initialization.
      FLOX_TCSH_INIT_SCRIPT="$(@coreutils@/bin/mktemp -p "$_FLOX_ACTIVATION_STATE_DIR")"
      generate_tcsh_startup_commands "$_flox_activate_tracelevel" "$_FLOX_ACTIVATION_STATE_DIR" "$PATH" "$MANPATH" "$_activate_d" "$FLOX_ENV" "${_FLOX_ACTIVATION_PROFILE_ONLY:-false}" > "$FLOX_TCSH_INIT_SCRIPT"
      # self destruct
      echo "@coreutils@/bin/rm '$FLOX_TCSH_INIT_SCRIPT'" >> "$FLOX_TCSH_INIT_SCRIPT"
      export FLOX_TCSH_INIT_SCRIPT

      exec "$_flox_shell" -m -c "$*"
    fi
    ;;
  *zsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -o NO_GLOBAL_RCS -o NO_RCS -c "$*"
    else
      if [ -n "${ZDOTDIR:-}" ]; then
        export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
      fi
      export ZDOTDIR="$_zdotdir"
      export FLOX_ZSH_INIT_SCRIPT="$_activate_d/zsh"
      # The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
      # automatically sourcing /etc/zshrc et al.
      exec "$_flox_shell" -o NO_GLOBAL_RCS -c "$*"
    fi
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac
