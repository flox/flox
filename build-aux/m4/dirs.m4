# -*- mode: autoconf; -*-
# ============================================================================ #
#
# @file m4/dirs.m4
#
# @brief Checks for the presence of various directories especially in the
#        `flox` repository.
#
#
# ---------------------------------------------------------------------------- #

#serial 1

# ---------------------------------------------------------------------------- #

# FLOX_INSTALL_DIRS
# -----------------
# Set the variables `bindir', `datadir', `includedir', `libdir', etc using
# conventional UPPER_CASE names.
AC_DEFUN([FLOX_INSTALL_DIRS],
[AC_SUBST([PREFIX], ["$prefix"])
AC_SUBST([BINDIR], ["$bindir"])
AC_SUBST([LIBDIR], ["$libdir"])
AC_SUBST([INCLUDEDIR], ["$includedir"])
AC_SUBST([DATADIR], ["$datadir"])
AC_SUBST([MANDIR], ["$mandir"])
AC_SUBST([PKGCONFIGDIR], ["$LIBDIR/pkgconfig"])
AC_SUBST([DOCDIR], ["$docdir"])
AC_SUBST([LIBEXECDIR], ["$libexecdir"])
AC_SUBST([SYSCONFDIR], ["$sysconfdir"])
# TODO: this needs to be setup so the relative `RPATH' works correctly.
AC_SUBST([INSTALL_TESTS_DIR], ["$DATADIR/flox/tests/bin"])
]) # FLOX_INSTALL_DIRS


# ---------------------------------------------------------------------------- #

# FLOX_ABS_SRCDIR
# ---------------
# Set the variable `abs_srcdir` to the absolute path to the directory containing
# the `configure' script.
AC_DEFUN([FLOX_ABS_SRCDIR],
[AC_SUBST([abs_srcdir], ["$( cd "$srcdir"; echo "$PWD"; )"])[]dnl
]) # FLOX_ABS_SRCDIR


# ---------------------------------------------------------------------------- #

# FLOX_INIT_DIRS
# --------------
# Initialize the variables associated with the various directories.
AC_DEFUN([FLOX_INIT_DIRS],
[AC_REQUIRE([FLOX_INSTALL_DIRS])dnl
AC_REQUIRE([FLOX_ABS_SRCDIR])dnl
]) # FLOX_INIT_DIRS


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
# vim: set filetype=config :
