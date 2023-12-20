# -*- mode: autoconf; -*-
# ============================================================================ #
#
# @file m4/nix.m4
#
# @brief Checks associated with `nix`.
#
#
# ---------------------------------------------------------------------------- #

#serial 1

# ---------------------------------------------------------------------------- #

# FLOX_CHECK_NIX_CC_WRAPPER
# -------------------------
# Detect whether `CC` is an executable or a shell script wrapper created
# by `nix`.
# Sets `CC_IS_NIX_WRAPPER` to `yes' if `CC` is a wrapper, `no' otherwise.
AC_DEFUN([FLOX_CHECK_NIX_WRAPPER],
[AC_CACHE_CHECK([whether CC is a nix wrapper], [flox_cv_nix_cc_wrapper],
  [AC_REQUIRE([FLOX_PROG_FILECMD])
   AC_REQUIRE([FLOX_PROG_CC])
   AC_REQUIRE([FLOX_PROG_GREP])
   AS_IF(
     [$FILECMD -Lb "$CC" 2>/dev/null[]dnl
        |$GREP -q "^a /nix/store/[[^ ]]*/bash script, ASCII text executable\$"],
     [flox_cv_nix_cc_wrapper=yes], [flox_cv_nix_cc_wrapper=no])])
AM_CONDITIONAL([CC_IS_NIX_WRAPPER], [test "$flox_cv_nix_cc_wrapper" = 'yes'])
]) # FLOX_CHECK_NIX_CC_WRAPPER


# ---------------------------------------------------------------------------- #

# FLOX_PROG_NIX
# -------------
# Set `NIX` to the path of the `nix` executable, if any.
# Set various `NIX_*` variables.
AC_DEFUN([FLOX_PROG_NIX], [
AC_ARG_VAR([NIX], [Purely functional package manager])
AC_PATH_PROG([NIX], [nix], [$MISSING nix])
]) # FLOX_PROG_NIX


# ---------------------------------------------------------------------------- #

# FLOX_CHECK_NIX_MODULES([ACTION-IF-FOUND], [ACTION-IF-NOT-FOUND])
# ----------------------
# Check whether the `nix` `pkg-config' modules are available, and set
# `NIX_CFLAGS' and `NIX_LIBS' accordingly.
# `-lnixfetchers' is added to `NIX_LIBS' if `nix-fetchers' is available.
# Checks for `nix-store nix-main nix-cmd nix-expr'.
AC_DEFUN([FLOX_CHECK_NIX_MODULES], [dnl
PKG_CHECK_MODULES(
  [NIX],
  [nix-store nix-main nix-cmd nix-expr],
  m4_default([$1], [:]),
  m4_default([$2],
             [AC_MSG_ERROR([Cannot find 'nix-{store|main|cmd|expr}.pc'])]))
  NIX_LIBS="-lnixfetchers $NIX_LIBS";
]) # FLOX_CHECK_NIX_MODULES


# ---------------------------------------------------------------------------- #

# FLOX_INHERIT_NIX_CONFIG_DEF(VAR, [EXPECT_VALUE])
# ------------------------------------------------
# Check whether `VAR` is defined in the `nix` configuration, and ( optionally )
# if it has the expected value.
# If `EXPECT_VALUE` is empty only check whether `VAR` is defined.
# If the check succeeds, invoke `AC_DEFINE(VAR, EXPECT_VALUE)'.
# XXX: We do NOT inherit the value of `VAR` from the `nix` configuration!
AC_DEFUN([FLOX_INHERIT_NIX_CONFIG_DEF], [
AC_REQUIRE([FLOX_CHECK_NIX_MODULES])
AC_MSG_CHECKING([nix configuration setting $1])
# Push current `CPPFLAGS' and language
_flox_v_inherit_nix_config_def_pushed_CPPFLAGS="$CPPFLAGS";
CPPFLAGS="$NIX_CPPFLAGS";
AC_LANG_PUSH([C++])
# Try to compile a program that includes `nix/config.h' and checks for `VAR'.
# NOTE: "@%:@" is replaced with "#" by `autoconf' to prevent `m4sh' from
#       treating it as a comment.
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef $1
     @%:@error "$1 is not defined"
   @%:@endif]m4_if(m4_default([$2], []), [], [], [
   @%:@if $1 != $2
     @%:@error "$1 is not set to $2"
   @%:@endif]))],
  [m4_if(m4_default([$2], []), [], [AC_DEFINE([$1])],
                                   [AC_DEFINE([$1], [$2])])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])
# Pop old `CPPFLAGS' and language
AC_LANG_POP
CPPFLAGS="$_flox_v_inherit_nix_config_def_pushed_CPPFLAGS";
]) # FLOX_INHERIT_NIX_CONFIG_DEF


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
