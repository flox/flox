# shellcheck shell=bash

# source_once <path>
# Sources specified file only once per shell invocation.
function source_once {
  local _guard_path=$1
  # 1) replace non-alphanumerics with _
  local _guard_id=${_guard_path//[^[:alnum:]]/_}
  # 2) collapse any double-underscores
  _guard_id=${_guard_id//__/_}
  # 3) trim leading/trailing _
  _guard_id=${_guard_id#_}
  _guard_id=${_guard_id%_}

  # 4) build guard name
  local _guard_var="__guard_${_guard_id}"

  # 5) test & set
  # Take particular care to obey Bash version 3 syntax here and avoid
  # the use of the `-v` test operator available in Bash v4+.
  if eval "[ -z \"\${${_guard_var}+x}\" ]"; then
    # create global (but not exported) var
    if declare -g "$_guard_var" &> /dev/null; then
      # declare -g is available
      declare -g "$_guard_var"=1
    else
      # fallback to simple assignment
      eval "$_guard_var=1"
    fi
    # No way to shellcheck user-provided code.
    # shellcheck disable=SC1090
    source "$_guard_path"
  fi
}
