# -*- mode: autoconf; -*-
# ============================================================================ #
#
# @file m4/progs.m4
#
# @brief Checks for common programs and tools, especially `coreutils`
#        and `findutils`.
#
#
# ---------------------------------------------------------------------------- #

#serial 1

# ---------------------------------------------------------------------------- #

# FLOX_PROG_MISSING
# -----------------
# Set `MISSING` to the path of the `missing` executable, if any.
# This should be located in the repo root's `build-aux` directory.
AC_DEFUN([FLOX_PROG_MISSING],
[AC_REQUIRE([FLOX_ABS_SRCDIR])
AC_ARG_VAR([MISSING], [how to invoke a missing program])
AC_PATH_PROG([MISSING], [missing], [AC_MSG_ERROR([missing not found])],
             [$PATH:$abs_srcdir/build-aux])
]) # FLOX_PROG_MISSING


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
