#!@nawk@/bin/nawk -f
#
# set-flox-env-vars.awk: grooms and emits commands to set Flox env variables
#
# Most notably, this script inspects the PATH and MANPATH variables and
# grooms them to put directories implied by the FLOX_ENV_DIRS variable
# at the front of the list. It also emits these variables in each of the
# supported [4] shell dialects, based on the value of the FLOX_SHELL
# environment variable.
#
# Usage:
#  awk -f set-flox-env-vars.awk - shell=path/to/<shell>
#
# Note that this script does not expect any input, and the entirety
# of its logic appears in the BEGIN block.

# Per the awk man page:
#   Parameters are local to the function; all other variables are global.
#   Thus local variables may be created by providing excess parameters in
#   the function definition.
#
# So the presence of "extra" parameters in the following function
# definition is intentional, and is used to create local variables.
function basename(file, # local variables follow
                  a,
		  n) {
  n = split(file, a, "/")
  return a[n]
}

function remove_path_dups(path, # local variables follow
                          path_array,
			  dedup_array,
			  seen_hash,
			  i,
			  dedup_count) {
  dedup_count = 1
  dedup_array[dedup_count] = ""
  for (i = 1; i <= split(path, path_array, ":"); i++) {
    if (seen_hash[path_array[i]] == 1) {
      continue
    } else {
      dedup_array[dedup_count++] = path_array[i]
      seen_hash[path_array[i]] = 1
    }
  }
  result = dedup_array[1]
  for (i = 2; i < dedup_count; i++) {
    result = result ":" dedup_array[i]
  }
  return "\"" result "\""
}

BEGIN {
  # Parse the command line arguments.
  if (ARGC != 2) {
    print "Usage: awk -f set-flox-env-vars.awk <shell>" > "/dev/stderr"
    exit 1
  }
  flox_shell = ARGV[1]
  _flox_shell = basename(flox_shell)

  # Calculate the values to be prepended to the PATH and MANPATH variables.
  # Start with the current value of FLOX_ENV.
  _prepend_path = \
    ENVIRON["FLOX_ENV"] "/bin:" \
    ENVIRON["FLOX_ENV"] "/sbin:"
  _prepend_manpath = \
    ENVIRON["FLOX_ENV"] "/share/man:"
  # Then add directories found in FLOX_ENV_DIRS. Don't worry about duplicating
  # the FLOX_ENV directory as remove_path_dups will remove dups later.
  for (i = 1; i < split(ENVIRON["FLOX_ENV_DIRS"], flox_env_dirs, ":"); i++) {
    _prepend_path = _prepend_path \
      flox_env_dirs[i] "/bin:" \
      flox_env_dirs[i] "/sbin:"
    _prepend_manpath = _prepend_manpath \
      flox_env_dirs[i] "/share/man:"
  }

  # Calculate the new PATH environment variable.
  new_path = remove_path_dups(_prepend_path ENVIRON["PATH"])

  # Calculate the new man(1) search path.
  #
  # Note that we always add a trailing colon to the MANPATH because
  # the search path for manual pages is somewhat complex:
  #
  # 1) If MANPATH begins with a colon, it is appended to the default list;
  # 2) if it ends with a colon, it is prepended to the default list;
  # 3) or if it contains two adjacent colons,
  #    the standard search path is inserted between the colons.
  # 4) else it overrides the standard search path.
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
  new_manpath = remove_path_dups(_prepend_manpath ENVIRON["MANPATH"] ":")

  if (_flox_shell == "bash") {
    print "export PATH=" new_path
    print "export MANPATH=" new_manpath
  } else if (_flox_shell == "tcsh") {
    print "setenv PATH " new_path
    print "setenv MANPATH " new_manpath
  } else if (_flox_shell == "fish") {
    print "set -gx PATH " new_path
    print "set -gx MANPATH " new_manpath
  } else if (_flox_shell == "zsh") {
    print "export PATH=" new_path
    print "export MANPATH=" new_manpath
  } else {
    print "Unknown shell: " flox_shell > "/dev/stderr"
    exit 1
  }
}
# No input block means script will exit immediately after BEGIN block.
