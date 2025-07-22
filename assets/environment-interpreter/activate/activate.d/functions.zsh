# shellcheck shell=zsh

# source_once <path>
# Sources specified file only once per shell invocation.
function source_once {
  local _guard_path=$1

  # normalize _guard_path → alnum+ underscores
  local _guard_id=${_guard_path//[^[:alnum:]]/_}
  _guard_id=${_guard_id//__/_}
  _guard_id=${_guard_id#_}; _guard_id=${_guard_id%_}

  # the guard variable name
  local _guard_var="__guard_${_guard_id}"

  # if not yet set → set and source the file
  if (( ! ${${(P)_guard_var}:-0} )); then
    typeset -g "$_guard_var"=1
    source "$_guard_path"
  fi
}
