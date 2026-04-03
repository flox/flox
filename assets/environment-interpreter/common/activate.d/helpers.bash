# shellcheck shell=bash
# shellcheck disable=SC2154 # _flox_activate_tracer is set by the caller (activate script)

_flox_activations="@flox_activations@"

# source_profile_d <profile.d directory> <profile variable mode> <FLOX_ENV_DIRS>
#
# source all scripts in <profile.d directory>
# FLOX_ENV_DIRS may be empty when in set mode
function source_profile_d {
  local _profile_d="${1?}"
  shift
  local _profile_variable_mode="${1?}"
  shift
  local _flox_env_dirs="${1?}"
  shift

  # make sure the directory exists
  [ -d "$_profile_d" ] || {
    echo "'$_profile_d' is not a directory" >&2
    return 1
  }

  # Use direct glob instead of subshell to avoid fork overhead
  local _saved_nullglob
  _saved_nullglob=$(shopt -p nullglob || true)
  shopt -s nullglob
  local _profile_scripts=("$_profile_d"/*.sh)
  eval "$_saved_nullglob"

  local _script
  for _script in "${_profile_scripts[@]}"; do
    "$_flox_activate_tracer" "$_script" START
    # shellcheck disable=SC1090 # from rendered environment
    source "$_script"
    "$_flox_activate_tracer" "$_script" END
  done

  # Language-specific setup (inlined from profile.d.functions to avoid
  # an extra source call)
  "$_flox_activate_tracer" "setup_python" START
  _setup_python "$_profile_variable_mode" "$_flox_env_dirs"
  "$_flox_activate_tracer" "setup_python" END
  "$_flox_activate_tracer" "setup_cmake" START
  _setup_cmake "$_profile_variable_mode" "$_flox_env_dirs"
  "$_flox_activate_tracer" "setup_cmake" END

  "$_flox_activate_tracer" "source_profile_d" END
}

# Setup Python3
function _setup_python {
  local _profile_variable_mode="${1?}"
  shift
  local _flox_env_dirs="${1?}"
  shift
  if [[ "$_profile_variable_mode" != "set" && -z "$_flox_env_dirs" ]]; then
    echo "Error: _flox_env_dirs cannot be empty when not in 'set' mode" >&2
    exit 1
  fi

  # Only run if `python3' is in `PATH'
  if [[ -x "$FLOX_ENV/bin/python3" ]]; then
    # Get the major/minor version from `python3' to determine the correct path.
    _python_version="$("$FLOX_ENV/bin/python3" -c 'import sys; print( "{}.{}".format( sys.version_info[0], sys.version_info[1] ) )')"
    _env_suffix="lib/python${_python_version}/site-packages"
    if [ "$_profile_variable_mode" = "set" ]; then
      PYTHONPATH="$FLOX_ENV/$_env_suffix"
    else
      PYTHONPATH="$($_flox_activations prepend-and-dedup --env-dirs "$FLOX_ENV_DIRS" --suffix "$_env_suffix" --path-like "${PYTHONPATH:-}")"
    fi
    export PYTHONPATH
  fi

  # Only run if `pip' is in `PATH' for non-containerize activations.
  if [[ (-x "$FLOX_ENV/bin/pip3") && (-n "${FLOX_ENV_PROJECT:-}") ]]; then
    PIP_CONFIG_FILE="$FLOX_ENV_PROJECT/.flox/pip.ini"
    export PIP_CONFIG_FILE
    cat > "$PIP_CONFIG_FILE" << EOF
  [global]
  require-virtualenv = true
EOF
  fi
}

# cmake requires the CMAKE_PREFIX_PATH variable in order to locate libraries
# and include files
function _setup_cmake {
  local _profile_variable_mode="${1?}"
  shift
  local _flox_env_dirs="${1?}"
  shift
  if [[ "$_profile_variable_mode" != "set" && -z "$_flox_env_dirs" ]]; then
    echo "Error: _flox_env_dirs cannot be empty when not in 'set' mode" >&2
    exit 1
  fi

  # Only run if `cmake' is installed to the environment.
  if [[ -x "$FLOX_ENV/bin/cmake" ]]; then
    if [ "$_profile_variable_mode" = "set" ]; then
      CMAKE_PREFIX_PATH="$FLOX_ENV"
    else
      CMAKE_PREFIX_PATH="$($_flox_activations prepend-and-dedup --env-dirs "$_flox_env_dirs" --path-like "${CMAKE_PREFIX_PATH:-}")"
    fi
    export CMAKE_PREFIX_PATH
  fi
}

# set_manifest_vars <flox_env>
#
# Set static environment variables from the manifest.
function set_manifest_vars {
  local _flox_env="${1?}"
  if [ -f "$_flox_env/activate.d/envrc" ]; then
    # shellcheck disable=SC1091 # from rendered environment
    source "$_flox_env/activate.d/envrc"
  fi
}
