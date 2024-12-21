"$_flox_activate_tracer" "$_activate_d/set-run-variables.bash" START

# Now that we support attaching to an environment we can no longer rely on
# the environment variable replay for setting the PATH and MANPATH variables,
# and must instead infer them from the FLOX_ENV_DIRS variable maintained for
# us by the flox CLI.

# Set IFS=: for this portion of the script.
_save_IFS="$IFS"
IFS=":"

# Get an iterable array of FLOX_ENV_DIRS.
declare -a _FLOX_ENV_DIRS
# If there's an outer activation with the CLI followed by an inner activation
# with just the activate script (this could happen e.g. for a build),
# we need to combine $FLOX_ENV and $FLOX_ENV_DIRS.
# If $FLOX_ENV is already in $FLOX_ENV_DIRS, the deduplication logic below will
# handle that
# shellcheck disable=SC2206
_FLOX_ENV_DIRS=($FLOX_ENV ${FLOX_ENV_DIRS:-})

# Set the PATH environment variable.
declare _prepend_path=""
for i in "${_FLOX_ENV_DIRS[@]}"; do
  _prepend_path="$_prepend_path${_prepend_path:+:}$i/bin:$i/sbin"
done
PATH="$_prepend_path${PATH:+:$PATH}"

# Set the man(1) search path.
# The search path for manual pages is determined
# from the MANPATH environment variable in a non-standard way:
#
# 1) If MANPATH begins with a colon, it is appended to the default list;
# 2) if it ends with a colon, it is prepended to the default list;
# 3) or if it contains two adjacent colons,
#    the standard search path is inserted between the colons.
# 4) If none of these conditions are met, it overrides the standard search path.
#
# In order for man(1) to find manual pages not defined in the flox environment,
# we ensure that we prepend the flox search path _with_ a colon in all cases.
#
# Thus, the man pages defined in the flox environment are searched first,
# and default search paths still apply.
# Additionally, decisions made by the user by setting the MANPATH variable
# are not overridden by the flox environment:
# - If MANPATH starts with `:` we now have `::` -> rule 1/3,
#   the defaults are inserted in between,
#   i.e. in front of MANPATH, but FLOXENV will take precedence in any case
# - If MANPATH ends with `:` we end with `:` -> rule 2,
#   the defaults are appended (no change)
# - If MANPATH does not start or end with `:`, -> rule 4,
#   FLOX_ENV:MANPATH replaces the defaults (no change)
declare _prepend_manpath=""
for i in "${_FLOX_ENV_DIRS[@]}"; do
  _prepend_manpath="$_prepend_manpath${_prepend_manpath:+:}$i/share/man"
done
MANPATH="$_prepend_manpath:${MANPATH:+$MANPATH}"

# Restore IFS.
IFS="$_save_IFS"
unset _save_IFS

# Remove duplicates from PATH and MANPATH. Note we must use `echo` and not
# `echo -n` in the command below so that trailing ":" characters are followed
# by a newline and treated by awk as an empty field.
declare _awkScript _nodup_PATH _nodup_MANPATH
# shellcheck disable=SC2016
_awkScript='BEGIN { RS = ":"; } { if (A[$0]) {} else { A[$0]=1; printf(((NR==1) ? "" : ":") $0); } }'
_nodup_PATH="$(echo "$PATH" | "$_nawk" "$_awkScript")"
_nodup_MANPATH="$(echo "$MANPATH" | "$_nawk" "$_awkScript")"
export PATH="${_nodup_PATH}"
export MANPATH="${_nodup_MANPATH}"

"$_flox_activate_tracer" "$_activate_d/set-run-variables.bash" END
