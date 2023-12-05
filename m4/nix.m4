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

# AK_CHECK_NIX
# ------------
# Detect whether `CC` is an executable or a shell script wrapper created
# by `nix`.
AC_DEFUN([AK_CHECK_NIX],
[AC_CACHE_CHECK([whether CC is a nix wrapper], [ak_cv_nix_cc_wrapper],
  [AC_REQUIRE([AC_PROG_CC])
  AS_IF(
    [file -Lb `which $CC` 2>/dev/null[]dnl
     |$GREP -q "^a /nix/store/[[^ ]]*/bash script, ASCII text executable\$"],
    [ak_cv_nix_cc_wrapper=yes], [ak_cv_nix_cc_wrapper=no])
  ])
AM_CONDITIONAL([NIX_CC_WRAPPER], [test "$ak_cv_nix_cc_wrapper" = 'yes'])
]) # AK_CHECK_NIX


# ---------------------------------------------------------------------------- #

# AK_PROG_NIX
# -----------
# Set `NIX` to the path of the `nix` executable, if any.
# Set various `NIX_*` variables.
AC_DEFUN([AK_PROG_NIX],
[AC_PATH_PROG([NIX], [nix], [$MISSING nix])
]) # AK_PROG_NIX


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
