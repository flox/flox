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

  declare -a _profile_scripts
  read -r -a _profile_scripts < <(
    cd "$_profile_d" || exit
    shopt -s nullglob
    echo *.sh
  )
  for profile_script in "${_profile_scripts[@]}"; do
    # shellcheck disable=SC1090 # from rendered environment
    source "$_profile_d/$profile_script"
  done

  # shellcheck disable=SC1091 # from rendered environment
  source "$_profile_d/profile.d.functions"
  setup_python "$_profile_variable_mode" "$_flox_env_dirs"
  unset -f setup_python
  setup_cmake "$_profile_variable_mode" "$_flox_env_dirs"
  unset -f setup_cmake
}

# flox_prepend_path <dir>
#
# Prepend <dir> to PATH and register it against $FLOX_ENV in
# _FLOX_ENV_PATH_PREPENDS so that later invocations of
# 'flox-activations fix-paths' replay the prepend at that environment's
# position in the layered PATH rather than demoting it behind the
# activated environments' bin directories. Intended for use by
# etc/profile.d scripts.
#
# <dir> must not contain ':' (illegal in PATH anyway) or '=' — entries
# are stored as colon-separated "<env>=<dir>" pairs without escaping.
function flox_prepend_path {
  local _dir="${1?}"
  local _env="${FLOX_ENV?}"
  case ":${_FLOX_ENV_PATH_PREPENDS:-}:" in
    # Already registered: within an activation a registration is always
    # accompanied by its PATH entry (set together below, or inherited
    # along with the PATH that already contains the dir), so skip the
    # prepend as well to keep repeated calls from duplicating it.
    *":$_env=$_dir:"*) : ;;
    *)
      export _FLOX_ENV_PATH_PREPENDS="$_env=$_dir${_FLOX_ENV_PATH_PREPENDS:+:$_FLOX_ENV_PATH_PREPENDS}"
      # Take effect immediately in this shell; subsequent fix-paths
      # invocations keep the dir ordered with its environment's layer.
      PATH="$_dir:$PATH"
      export PATH
      ;;
  esac
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
