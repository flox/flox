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
AC_PATH_PROG([CC], ["${CC:-cc}"], [$MISSING cc])
]) # FLOX_PROG_CC


# ---------------------------------------------------------------------------- #

# FLOX_PROG_GREP
# --------------
# Set `GREP` to the path of the `grep` executable, if any.
AC_DEFUN([FLOX_PROG_GREP], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_REQUIRE([AC_PROG_GREP])
AC_PATH_PROG([GREP], ["${GREP:-grep}"], [$MISSING grep])
]) # FLOX_PROG_GREP


# ---------------------------------------------------------------------------- #

# FLOX_PROG_FILE
# --------------
# Set `FILE` to the path of the `file` executable, if any.
AC_DEFUN([FLOX_PROG_FILECMD], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([FILECMD], [File type recognizer])
AC_PATH_PROG([FILECMD], [file], [$MISSING file])
]) # FLOX_PROG_FILE


# ---------------------------------------------------------------------------- #

# FLOX_PROG_MKTEMP
# ----------------
AC_DEFUN([FLOX_PROG_MKTEMP], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([MKTEMP], [Create temporary files and directories])
AC_PATH_PROG([MKTEMP], [mktemp], [$MISSING mktemp])
]) # FLOX_PROG_MKTEMP


# ---------------------------------------------------------------------------- #

# FLOX_PROG_REALPATH
# ----------------
AC_DEFUN([FLOX_PROG_REALPATH], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([REALPATH], [Canonicalize paths])
AC_PATH_PROG([REALPATH], [realpath], [$MISSING realpath])
]) # FLOX_PROG_REALPATH


# ---------------------------------------------------------------------------- #

# FLOX_PROG_MKDIR
# ---------------
# Declares `MKDIR' and `MKDIR_P' variables.
AC_DEFUN([FLOX_PROG_MKDIR], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([MKDIR], [Create directories])
AC_PATH_PROG([MKDIR], [mkdir], [$MISSING mkdir])
AC_SUBST([MKDIR_P], ["$MKDIR -p"])
]) # FLOX_PROG_MKTEMP


# ---------------------------------------------------------------------------- #

# FLOX_PROG_LN
# ------------
# Declares `LN' and `LN_S' variables.
AC_DEFUN([FLOX_PROG_LN], [
AC_REQUIRE([FLOX_PROG_MISSING])
AC_ARG_VAR([LN], [Create symbolic links])
AC_PATH_PROG([LN], [ln], [$MISSING ln])
AC_SUBST([LN_S], ["$LN -s"])
]) # FLOX_PROG_LN


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
