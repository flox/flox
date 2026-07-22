# jq for flox_plugin_data below. Neither the activate script's `$_jq` nor
# any PATH lookup is guaranteed here: the wrapper (build-mode) context never
# sets `$_jq`, and this file is sourced before PATH is finalized in both
# contexts. Substituted at build time, like the other tool paths used across
# activate.d.
_jq="@jq@/bin/jq"

# flox_plugin_data <plugin-package-name>
#
# Print the named plugin's manifest data ([plugins.<pkg-name>]) as compact
# JSON, read from the activating environment's manifest.lock. Errors (and
# thereby blocks activation, which runs under `set -e`) when the lockfile
# is missing or holds no data for the plugin: a plugin script calling this
# expects its table to exist, and activating without it would only defer
# the failure to first use.
function flox_plugin_data {
  local _plugin_name="${1?}"
  local _lockfile="$FLOX_ENV/manifest.lock"
  if [ ! -f "$_lockfile" ]; then
    echo "flox_plugin_data: no lockfile at '$_lockfile'." >&2
    return 1
  fi
  local _data
  # shellcheck disable=SC2016 # $plugin is a jq variable, not shell expansion
  if ! _data="$("$_jq" -c --arg plugin "$_plugin_name" \
    '.manifest.plugins[$plugin] // empty' "$_lockfile")"; then
    echo "flox_plugin_data: could not read plugin data from '$_lockfile'." >&2
    return 1
  fi
  if [ -z "$_data" ]; then
    echo "flox_plugin_data: no [plugins.$_plugin_name] data in '$_lockfile'." >&2
    return 1
  fi
  echo "$_data"
}

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
