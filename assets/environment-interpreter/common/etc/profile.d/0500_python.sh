# shellcheck shell=bash
_cat="@coreutils@/bin/cat"
_realpath="@coreutils@/bin/realpath"

# FIXME: this belongs in a helper-functions.bash file as suggested in #1767
function removePathDups {
  for __varname in "$@"; do
    declare -A __seen
    __rewrite=

    # Split $PATH on ':' into the array '__paths'
    IFS=: read -r -a __paths <<< "${!__varname}"
    # Unset IFS to its default value
    unset IFS

    for __dir in "${__paths[@]}"; do
      if [ -z "${__seen[$__dir]:-}" ]; then
        __rewrite="$__rewrite${__rewrite:+:}$__dir"
        __seen[$__dir]=1
      fi
    done

    export "$__varname"="$__rewrite"
    unset __seen
    unset __rewrite
  done
}

# ============================================================================ #
#
# Setup Python3
#
# ---------------------------------------------------------------------------- #

# Only run if `python3' is in `PATH'
if [[ -x "$FLOX_ENV/bin/python3" ]]; then
  # Get the major/minor version from `python3' to determine the correct path.
  _env_pypath="$FLOX_ENV/lib/python$(
    "$FLOX_ENV/bin/python3" -c 'import sys
print( "{}.{}".format( sys.version_info[0], sys.version_info[1] ) )'
  )/site-packages"

  # Always prepend to the PYTHONPATH.
  PYTHONPATH="$_env_pypath${PYTHONPATH:+:}${PYTHONPATH:-}"

  # Remove duplicate elements in case the path was already present.
  removePathDups PYTHONPATH

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
