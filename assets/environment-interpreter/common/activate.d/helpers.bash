# jq for flox_plugin_data below. A PATH lookup is not guaranteed here: this
# file is sourced before PATH is finalized in both the activate and the
# wrapper (build-mode) contexts. Substituted at build time, like the other
# tool paths used across activate.d.
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
  printf '%s\n' "$_data"
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

# source_plugin_profile_d <env profile.d directory> <interpreter profile.d directory>
#
# Source the profile.d scripts provided by installed packages (plugins):
# the scripts in the environment's merged profile.d that the interpreter
# does not itself provide. The interpreter's own scripts are sourced
# separately from the interpreter package, before [vars] are exported,
# so that [vars] can override their variables while remaining readable
# by plugin scripts.
function source_plugin_profile_d {
  local _profile_d="${1?}"
  shift
  local _interpreter_profile_d="${1?}"
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
    if [ ! -e "$_interpreter_profile_d/$profile_script" ]; then
      # shellcheck disable=SC1090 # from rendered environment
      source "$_profile_d/$profile_script"
    fi
  done
}

# set_manifest_vars <flox_env>
#
# Set static environment variables from the manifest.
#
# The envrc of an environment rendered by flox before 1.19.1 opens by
# defaulting SSL_CERT_FILE to the environment's CA bundle and copying it
# into NIX_SSL_CERT_FILE. Those bundles include CAs that have since been
# compromised, and an environment keeps its envrc until it is next
# rebuilt, so the default is undone after sourcing. flox exports
# NIX_SSL_CERT_FILE ahead of this script and leaves SSL_CERT_FILE alone,
# so an SSL_CERT_FILE that was empty before sourcing and differs from
# NIX_SSL_CERT_FILE afterwards is the envrc's default and is unset. One
# the user had already set is left in place: the default never replaced
# it, and flox then sets no NIX_SSL_CERT_FILE, so the envrc copies the
# user's value and the two agree.
function set_manifest_vars {
  local _flox_env="${1?}"
  local _envrc="$_flox_env/activate.d/envrc"
  [ -f "$_envrc" ] || return 0

  local _ssl_cert_file_before="${SSL_CERT_FILE-}"
  # shellcheck disable=SC1090,SC1091 # from rendered environment
  source "$_envrc"
  if [ -z "$_ssl_cert_file_before" ] && [ -n "${SSL_CERT_FILE-}" ] \
    && [ -n "${NIX_SSL_CERT_FILE-}" ] && [ "$SSL_CERT_FILE" != "$NIX_SSL_CERT_FILE" ]; then
    unset SSL_CERT_FILE
  fi
}
