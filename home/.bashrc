echo "sourcing .bashrc"
echo "${_flox_already_sourcing_rc:-empty}"
export PS1="myprompt>"
eval "$(/Users/zmitchell/src/flox/double-bashrc-exec/cli/target/debug/flox activate -m run -d "$PWD")"
