# shellcheck shell=bash
export _coreutils="@coreutils@"
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

# Only run if `pip' is in `PATH'
if [[ -x "$FLOX_ENV/bin/pip3" ]]; then
  PIP_CONFIG_FILE="$("$_coreutils/bin/realpath" --no-symlinks "$FLOX_ENV/../../pip.ini")"
  export PIP_CONFIG_FILE
  "$_coreutils/bin/cat" > "$PIP_CONFIG_FILE" << EOF
[global]
require-virtualenv = true
EOF
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
