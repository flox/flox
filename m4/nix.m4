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
  read -N4 MAGIC < "$( command -v $CC; )";
  # Drop the first character, which is '\0'
dnl The sequence `@%:@' expands to `#' when processed by `m4'
  AS_IF([test "${MAGIC@%:@?}" = 'ELF'],
        [ak_cv_nix_cc_wrapper=no],
        [ak_cv_nix_cc_wrapper=yes])
  ])
AM_CONDITIONAL([NIX_CC_WRAPPER], [test "x$ak_cv_nix_cc_wrapper" = xyes])
])# AK_CHECK_NIX


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
