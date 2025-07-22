# shellcheck shell=tcsh

# source_once <path>
# Sources specified file only once per shell invocation.

# Note that the continuation character within a tcsh alias must
# be a double backslash as described in the "writing long aliases"
# section of the following document:
#
# https://home.adelphi.edu/sbloch/class/archive/271/fall2005/notes/aliases.html

alias source_once \
 'set _guard_path = \!:1; \\
  set _guard_id = `echo "$_guard_path" | @gnused@/bin/sed -E '"'"'s/[^A-Za-z0-9]/_/g; s/_+/_/g; s/^_//; s/_$//'"'"'`; \\
  set _guard_var = "__guard_$_guard_id"; \\
  if (`eval echo \$\?$_guard_var` == 0) source $_guard_path; \\
  set $_guard_var = 1'
