# "in-place" mode: emit activation commands in correct shell dialect by echoing
# the contents of the shell-specific activation script.  N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
case "$_flox_shell" in
  *bash)
    echo "export _flox_activate_tracelevel=\"$_flox_activate_tracelevel\";"
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    echo "export _FLOX_ACTIVATION_STATE_DIR=\"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "source '$FLOX_ENV/activate.d/bash';"
    ;;
  *fish)
    echo "set -gx _flox_activate_tracelevel \"$_flox_activate_tracelevel\";"
    echo "set -gx FLOX_ENV \"$FLOX_ENV\";"
    echo "set -gx _FLOX_ACTIVATION_STATE_DIR \"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "source '$FLOX_ENV/activate.d/fish';"
    ;;
  *tcsh)
    echo "setenv _flox_activate_tracelevel \"$_flox_activate_tracelevel\";"
    echo "setenv FLOX_ENV \"$FLOX_ENV\";"
    echo "setenv _FLOX_ACTIVATION_STATE_DIR \"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "source '$FLOX_ENV/activate.d/tcsh';"
    ;;
  # Any additions should probably be restored in zdotdir/* scripts
  *zsh)
    echo "export _flox_activate_tracelevel=\"$_flox_activate_tracelevel\";"
    echo "export FLOX_ENV=\"$FLOX_ENV\";"
    if [ -n "${ZDOTDIR:-}" ]; then
      echo "export FLOX_ORIG_ZDOTDIR=\"$ZDOTDIR\";"
    fi
    echo "export ZDOTDIR=\"$_zdotdir\";"
    echo "export FLOX_ZSH_INIT_SCRIPT=\"$FLOX_ENV/activate.d/zsh\";"
    echo "export _FLOX_ACTIVATION_STATE_DIR=\"$_FLOX_ACTIVATION_STATE_DIR\";"
    echo "source '$FLOX_ENV/activate.d/zsh';"
    ;;
  *)
    echo "Unsupported shell: $_flox_shell" >&2
    exit 1
    ;;
esac
