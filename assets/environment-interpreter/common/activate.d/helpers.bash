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
