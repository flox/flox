# "interactive" mode: invoke the user's shell with args that:
#   a. defeat the shell's normal startup scripts
#   b. source the relevant activation script
case "$_flox_shell" in
  *bash)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" --noprofile --norc
    else
      if [ -t 1 ]; then
        exec "$_flox_shell" --noprofile --rcfile "$FLOX_ENV/activate.d/bash"
      else
        # The bash --rcfile option only works for interactive shells
        # so we need to cobble together our own means of sourcing our
        # startup script for non-interactive shells.
        # XXX Is this case even a thing? What's the point of activating with
        #     no command to be invoked and no controlling terminal from which
        #     to issue commands?!? A broken docker experience maybe?!?
        exec "$_flox_shell" --noprofile --norc -s <<< "source $FLOX_ENV/activate.d/bash"
      fi
    fi
    ;;
  *fish)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell"
    else
      exec "$_flox_shell" --init-command "set -gx _flox_activate_tracelevel $_flox_activate_tracelevel; source $FLOX_ENV/activate.d/fish"
    fi
    ;;
  *tcsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -f
    else
      export FLOX_ORIG_HOME="$HOME"
      export HOME="$_tcsh_home"
      export FLOX_TCSH_INIT_SCRIPT="$FLOX_ENV/activate.d/tcsh"
      # The -m option is required for tcsh to source a .tcshrc file that
      # the effective user does not own.
      exec "$_flox_shell" -m
    fi
    ;;
  *zsh)
    if [ -n "$FLOX_NOPROFILE" ]; then
      exec "$_flox_shell" -o NO_GLOBAL_RCS -o NO_RCS
    else
      export FLOX_ORIG_ZDOTDIR="$ZDOTDIR"
      export ZDOTDIR="$_zdotdir"
      export FLOX_ZSH_INIT_SCRIPT="$FLOX_ENV/activate.d/zsh"
      # The "NO_GLOBAL_RCS" option is necessary to prevent zsh from
      # automatically sourcing /etc/zshrc et al.
      exec "$_flox_shell" -o NO_GLOBAL_RCS
    fi
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac
