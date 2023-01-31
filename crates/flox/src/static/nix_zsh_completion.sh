
function _nix() {
  local ifs_bk="$IFS"
  local input=("${(Q)words[@]:1}")
  # echo $input
  # echo $CURRENT
  IFS=$'\n'
  local res=($(NIX_GET_COMPLETIONS=$((CURRENT - OFFSET - 2)) "$input[@]" 2>/dev/null))
  IFS="$ifs_bk"
  local tpe="${${res[1]}%%>	*}"
  local -a suggestions
  declare -a suggestions
  for suggestion in ${res:1}; do
    # FIXME: This doesn't work properly if the suggestion word contains a `:`
    # itself
    suggestions+="${suggestion/	/:}"
  done
  if [[ "$tpe" == filenames ]]; then
    compadd -f
  fi
  _describe 'nix' suggestions
}

_nix
