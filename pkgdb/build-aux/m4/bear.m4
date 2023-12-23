# -*- mode: autoconf; -*-
# ============================================================================ #
#
# @file m4/bear.m4
#
# @brief Checks and utilities associated with `bear`
#        ( LLVM build analysis tool ).
#
#
# ---------------------------------------------------------------------------- #

#serial 1

# ---------------------------------------------------------------------------- #

# FLOX_PROG_BEAR
# --------------
AC_DEFUN([FLOX_PROG_BEAR], [
AC_ARG_VAR([BEAR], [Build EAR ( LLVM build analysis tool )])
AC_PATH_PROG([BEAR], [bear], [$MISSING bear])
]) # FLOX_PROG_BEAR


# ---------------------------------------------------------------------------- #

# FLOX_PROG_BEAR_WRAPPER
# ----------------------
# Locate LLVM EAR wrapper utility.
# This should be symlinked into the project root under `bear.d/c++' so that it
# may be used instead of the default `c++' compiler.
AC_DEFUN([FLOX_PROG_BEAR_WRAPPER], [
AC_ARG_VAR([BEAR_WRAPPER], [Build EAR wrapper utility])
_flox_v_bear_prefix="${BEAR%/bin/*}";
AC_SUBST([BEAR_WRAPPER], [$_flox_v_bear_prefix/lib/bear/wrapper])
]) # FLOX_PROG_BEAR_WRAPPER


# ---------------------------------------------------------------------------- #

# FLOX_CONFIG_BEAR_WRAPPER_CXX([INSTALL-DIR])
# -----------------------------------------
# Create a wrapper for the C++ compiler that will run through `bear'.
# This wrapper is created in the `INSTALL-DIR' directory.
AC_DEFUN([FLOX_CONFIG_BEAR_WRAPPER_CXX], [
AC_REQUIRE([FLOX_PROG_MKDIR])
AC_REQUIRE([FLOX_PROG_LN])
AC_REQUIRE([FLOX_PROG_BEAR_WRAPPER])
AC_ARG_VAR([BEAR_WRAPPER_CXX], [Build EAR wrapper utility for C++])
AS_CASE([$1], [/*], [BEAR_WRAPPER_CXX="$1/c++"],
                    [BEAR_WRAPPER_CXX="$PWD/$1/c++"])
AC_SUBST([BEAR_WRAPPER_CXX])
AC_CONFIG_COMMANDS([$1/c++],
  [$MKDIR_P "$1";
   $LN_S -f "$BEAR_WRAPPER" "$BEAR_WRAPPER_CXX";],
  [BEAR_WRAPPER="$BEAR_WRAPPER";
   BEAR_WRAPPER_CXX="$BEAR_WRAPPER_CXX"])
]) # FLOX_CONFIG_BEAR_WRAPPER_CXX


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
