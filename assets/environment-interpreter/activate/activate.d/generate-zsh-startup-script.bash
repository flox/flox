# shellcheck shell=bash
# shellcheck disable=SC2154

_sed="@gnused@/bin/sed"
_flox_activations="@flox_activations@"

# N.B. the output of
# these scripts may be eval'd with backticks which have the effect of removing
# newlines from the output, so we must ensure that the output is a valid shell
# script fragment when represented on a single line.
generate_zsh_startup_script() {
  "$_flox_activate_tracer" "generate_zsh_startup_script()" START

  _flox_activate_tracelevel="${1?}"
  echo "_flox_activate_tracelevel=\"$_flox_activate_tracelevel\";"
  shift
  _FLOX_ACTIVATION_STATE_DIR="${1?}"
  echo "_FLOX_ACTIVATION_STATE_DIR=\"$_FLOX_ACTIVATION_STATE_DIR\";"
  shift
  _activate_d="${1?}"
  echo "_activate_d=\"$_activate_d\";"
  shift
  _FLOX_ENV="${1?}"
  echo "_FLOX_ENV=\"$_FLOX_ENV\";"
  shift
  _FLOX_ENV_CACHE="${1?}"
  # This may be an empty string
  echo "_FLOX_ENV_CACHE=\"$_FLOX_ENV_CACHE\";"
  shift
  _FLOX_ENV_PROJECT="${1?}"
  # This may be an empty string
  echo "_FLOX_ENV_PROJECT=\"$_FLOX_ENV_PROJECT\";"
  shift
  _FLOX_ENV_DESCRIPTION="${1?}"
  # This may be an empty string
  echo "_FLOX_ENV_DESCRIPTION=\"$_FLOX_ENV_DESCRIPTION\";"
  shift

  echo "source $_activate_d/zsh;"

  "$_flox_activate_tracer" "generate_zsh_startup_script()" END
}
