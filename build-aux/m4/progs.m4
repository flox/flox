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

# FLOX_PROG_CC
# --------------
# Set `CC` to the path of the `cc` executable, if any.
AC_DEFUN([FLOX_PROG_CC], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_REQUIRE([AC_PROG_CC])
AC_PATH_PROG([CC], [$CC], [$MISSING cc])
]) # FLOX_PROG_CC


# ---------------------------------------------------------------------------- #

# FLOX_PROG_GREP
# --------------
# Set `GREP` to the path of the `grep` executable, if any.
AC_DEFUN([FLOX_PROG_GREP], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_REQUIRE([AC_PROG_GREP])
AC_PATH_PROG([GREP], [$GREP], [$MISSING grep])
]) # FLOX_PROG_GREP


# ---------------------------------------------------------------------------- #

# FLOX_PROG_FILE
# --------------
# Set `FILE` to the path of the `file` executable, if any.
AC_DEFUN([FLOX_PROG_FILE], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([FILE], [File type recognizer])
AC_PATH_PROG([FILE], [$FILE], [$MISSING file])
]) # FLOX_PROG_FILE


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
