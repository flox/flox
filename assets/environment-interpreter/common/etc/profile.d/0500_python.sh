# shellcheck shell=bash
_cat="@coreutils@/bin/cat"
_realpath="@coreutils@/bin/realpath"
_flox_activations="@flox_activations@"

# ============================================================================ #
#
# Setup Python3
#
# ---------------------------------------------------------------------------- #

# Only run if `python3' is in `PATH'
if [[ -x "$FLOX_ENV/bin/python3" ]]; then
  # Get the major/minor version from `python3' to determine the correct path.
  _python_version="$("$FLOX_ENV/bin/python3" -c 'import sys; print( "{}.{}".format( sys.version_info[0], sys.version_info[1] ) )')"
  # This will be appended to each environment directory to form an entry in the
  # PATH-like variable.
  _env_suffix="lib/python${_python_version}/site-packages"
  PYTHONPATH="$($_flox_activations prepend-and-dedup --env-dirs "$FLOX_ENV_DIRS" --suffix "$_env_suffix" --path-like "${PYTHONPATH:-}")"
  export PYTHONPATH
fi

# Only run if `pip' is in `PATH' for non-containerize activations.
# FLOX_ENV_PROJECT is unset in `containerize`, but *is* set for builds and
# other activations. We don't need a virtual environment inside a container.
if [[ (-x "$FLOX_ENV/bin/pip3") && (-n "${FLOX_ENV_PROJECT:-}") ]]; then
  PIP_CONFIG_FILE="$FLOX_ENV_PROJECT/.flox/pip.ini"
  export PIP_CONFIG_FILE
  "$_cat" > "$PIP_CONFIG_FILE" << EOF
[global]
require-virtualenv = true
EOF
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
