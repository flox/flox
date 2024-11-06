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

# "-c" command mode: pass both [2] arguments unaltered to shell invocation
case "$_flox_shell" in
  *bash)
    pwd
    if [ -d "$(pwd)" ]; then
      echo "exists"
      ls "$(pwd)"
    fi
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" --noprofile --norc -c "$*"
    else
      if [ -t 1 ]; then
        exec "$_flox_shell" --noprofile --rcfile "$_activate_d/bash" -c "$*"
      else
        # The bash --rcfile option only works for interactive shells
        # so we need to cobble together our own means of sourcing our
        # startup script for non-interactive shells.
        exec "$_flox_shell" --noprofile --norc -s <<< "source $_activate_d/bash && $*"
      fi
    fi
    ;;
  *fish)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$*"
    else
      exec "$_flox_shell" --init-command "set -gx _flox_activate_tracelevel $_flox_activate_tracelevel; source $_activate_d/fish" -c "$*"
    fi
    ;;
  *tcsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -c "$*"
    else
      export FLOX_ORIG_HOME="$HOME"
      export HOME="$_tcsh_home"
      export FLOX_TCSH_INIT_SCRIPT="$_activate_d/tcsh"
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
