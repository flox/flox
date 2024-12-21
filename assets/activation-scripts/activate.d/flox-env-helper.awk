#!@nawk@/bin/nawk -f
#
# flox-env-helper: grooms and emits commands to set Flox env variables
#
# This script inspects environment variables such as PATH and MANPATH and
# grooms them to put directories implied by the FLOX_ENV_DIRS variable
# at the front of the list. It also emits commands to set variables in each
# of the supported [4] shell dialects, based on the value of the FLOX_SHELL
# environment variable.
#
# Usage:
#  source <(flox-env-helper <shell>) # bash, zsh
#  flox-env-helper <shell> | source  # fish
#  eval "`flox-env-helper <shell>`"  # tcsh
#
# Note that this script does not expect any input, and the entirety of its
# logic appears in the BEGIN block.
#
# It was formatted using `gawk --pretty-print` and then hand-edited to change
# tabs back to two spaces and to add back newlines that it had removed from
# within functions.

BEGIN {
  # Parse the command line arguments.
  if (ARGC != 2) {
    print("Usage: flox-env-helper <shell>") > "/dev/stderr"
    exit 1
  }

  # Confirm that the mandatory FLOX_ENV variable is set.
  if (ENVIRON["FLOX_ENV"] == "") {
    print("flox-env-helper ERROR: FLOX_ENV is not set") > "/dev/stderr"
    exit 1
  }

  tracer("BEGIN")

  # Set the default value of FLOX_ENV_DIRS if it is not set.
  # First check to see if FLOX_ENV is already in FLOX_ENV_DIRS,
  # and if so then don't change the nesting order. This is particularly
  # important for the default environment which is reactivated in all
  # subshells.
  new_flox_env_dirs = prepend_if_not_found(ENVIRON["FLOX_ENV"], ENVIRON["FLOX_ENV_DIRS"])

  # The FLOX_ENV_LIB_DIRS variable is a direct translation of the FLOX_ENV_DIRS
  # variable but with each directory having "/lib" appended to it.
  new_flox_env_lib_dirs = append_dirs("/lib", new_flox_env_dirs)

  # Calculate the values to be prepended to the PATH and MANPATH variables.
  # First add directories found in FLOX_ENV_DIRS, then add the current value
  # of FLOX_ENV. Don't worry about duplicating the FLOX_ENV directory as
  # remove_path_dups will remove dups later.
  _prepend_path = ""
  _prepend_manpath = ""
  for (i = 1; i <= split(new_flox_env_dirs, flox_env_dirs, ":"); i++) {
    _prepend_path = _prepend_path flox_env_dirs[i] "/bin:" flox_env_dirs[i] "/sbin:"
    _prepend_manpath = _prepend_manpath flox_env_dirs[i] "/share/man:"
  }
  _prepend_path = _prepend_path ENVIRON["FLOX_ENV"] "/bin:" ENVIRON["FLOX_ENV"] "/sbin:"
  _prepend_manpath = _prepend_manpath ENVIRON["FLOX_ENV"] "/share/man:"

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

  # Emit commands to set variables for the various shell dialects.
  flox_shell_basename = basename(ARGV[1])
  if (flox_shell_basename == "bash") {
    print "export FLOX_ENV_DIRS=\"" new_flox_env_dirs "\";"
    print "export FLOX_ENV_LIB_DIRS=\"" new_flox_env_lib_dirs "\";"
    print "export PATH=\"" new_path "\";"
    print "export MANPATH=\"" new_manpath "\";"
  } else if (flox_shell_basename == "tcsh") {
    print "setenv FLOX_ENV_DIRS \"" new_flox_env_dirs "\";"
    print "setenv FLOX_ENV_LIB_DIRS \"" new_flox_env_lib_dirs "\";"
    print "setenv PATH \"" new_path "\";"
    print "setenv MANPATH \"" new_manpath "\";"
  } else if (flox_shell_basename == "fish") {
    print "set -gx FLOX_ENV_DIRS \"" new_flox_env_dirs "\";"
    print "set -gx FLOX_ENV_LIB_DIRS \"" new_flox_env_lib_dirs "\";"
    print "set -gx PATH \"" new_path "\";"
    print "set -gx MANPATH \"" new_manpath "\";"
  } else if (flox_shell_basename == "zsh") {
    print "export FLOX_ENV_DIRS=\"" new_flox_env_dirs "\";"
    print "export FLOX_ENV_LIB_DIRS=\"" new_flox_env_lib_dirs "\";"
    print "export PATH=\"" new_path "\";"
    print "export MANPATH=\"" new_manpath "\";"
  } else {
    print("Unknown shell: " ARGV[1]) > "/dev/stderr"
    exit 1
  }

  tracer("END")
}

# No input block means script will exit immediately after BEGIN block.

# Function definitions follow.
#
# A note regarding function arguments, per the awk man page:
#   Parameters are local to the function; all other variables are global.
#   Thus local variables may be created by providing excess parameters in
#   the function definition.
#
# So the presence of "extra" parameters in the following function
# definitions is intentional and for declaring local variables.

function append_dirs(string, path, _path_array, _result, i)
{
  # local variables: _path_array, _result, i
  _result = ""
  for (i = 1; i <= split(path, _path_array, ":"); i++) {
    if (_result == "") {
      _result = _path_array[i] string
    } else {
      _result = _result ":" _path_array[i] string
    }
  }
  return _result
}

function basename(file, _a, _n)
{
  # local variables: _a, _n
  _n = split(file, _a, "/")
  return _a[_n]
}

# Inserts provided value at beginning of path provided it is
# not already found in the path. This does not remove duplicates.
function prepend_if_not_found(insert, path, _path_array, _seen_hash, _result, i)
{
  # local variables: _path_array, _seen_hash, _result, i
  for (i = 1; i <= split(path, _path_array, ":"); i++) {
    _seen_hash[_path_array[i]] = 1
  }
  if (_seen_hash[insert] == 1) {
    _result = path
  } else if (path == "") {
    _result = insert
  } else {
    _result = insert ":" path
  }
  return _result
}

function remove_path_dups(path, _path_array, _dedup_array, _seen_hash, _dedup_count, _result, i)
{
  # local variables: _path_array, _dedup_array, _seen_hash, _dedup_count, _result, i
  _dedup_count = 1
  _dedup_array[_dedup_count] = ""
  for (i = 1; i <= split(path, _path_array, ":"); i++) {
    if (_seen_hash[_path_array[i]] == 1) {
      continue
    } else {
      _dedup_array[_dedup_count++] = _path_array[i]
      _seen_hash[_path_array[i]] = 1
    }
  }
  _result = _dedup_array[1]
  for (i = 2; i < _dedup_count; i++) {
    _result = _result ":" _dedup_array[i]
  }
  return _result
}

# Helper function for invoking the Flox tracer.
function tracer(label)
{
  if (ENVIRON["_flox_activate_tracer"] != "") {
    printf "%s %s %s %s;\n", ENVIRON["_flox_activate_tracer"], ENVIRON["_flox_env_helper"], ARGV[1], label
  }
}
