# source_profile_d <profile.d directory>
#
# source all scripts in <profile.d directory>
function source_profile_d {
  local _profile_d="${1?}"
  shift

  # make sure the directory exists
  [ -d "$_profile_d" ] || {
    echo "'$_profile_d' is not a directory" >&2
    return 1
  }

  declare -a _profile_scripts
  # TODO: figure out why this is needed
  set +e
  read -r -d '' -a _profile_scripts < <(
    cd "$_profile_d" || exit
    shopt -s nullglob
    echo *.sh
  )
  set -e
  for profile_script in "${_profile_scripts[@]}"; do
    # shellcheck disable=SC1090 # from rendered environment
    source "$_profile_d/$profile_script"
  done
}
