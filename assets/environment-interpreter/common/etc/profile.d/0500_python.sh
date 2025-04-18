# shellcheck shell=bash
_cat="@coreutils@/bin/cat"
_realpath="@coreutils@/bin/realpath"

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
  # Only add the path if its missing
  case ":${PYTHONPATH:-}:" in
    *:"$_env_pypath":*) : ;;
    *) PYTHONPATH="${PYTHONPATH:+$PYTHONPATH:}$_env_pypath" ;;
  esac
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
