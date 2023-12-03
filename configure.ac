# -*- Autoconf -*-
# ============================================================================ #
#
# Process this file with autoconf to produce a configure script.
#
#
# ---------------------------------------------------------------------------- #

AC_PREREQ( [2.71] )

# TODO: use different email address
AC_INIT([flox],
        m4_syscmd([grep version .cz.toml|cut -d'"' -f2]),
        [alex.ameen@floxdev.com])

AC_CONFIG_MACRO_DIRS([m4])
AC_CONFIG_AUX_DIR([build-aux])

AM_INIT_AUTOMAKE([-Wall -Werror foreign subdir-objects])

AC_CONFIG_SRCDIR([pkgdb/src/main.cc])
#AC_CONFIG_HEADERS([pkgdb/config.h])


# ---------------------------------------------------------------------------- #

# Checks for programs.

AC_PROG_CXX
AC_PROG_CC
AC_PROG_INSTALL
AC_LN_S


# ---------------------------------------------------------------------------- #

# Checks for libraries.

AC_CHECK_LIB(
  [nixfetchers]dnl
, translit( [_ZN3nix8fetchers11attrsToJSONERKSt3mapINSt7]
            [__cxx1112basic_stringIcSt11char_traitsIcESaIcEEESt7variantIJS7]
            [_mNS_8ExplicitIbEEEESt4lessIS7_ESaISt4pairIKS7_SB_EEE]
          , ' '
          )
)


# ---------------------------------------------------------------------------- #

# Checks for header files.

AC_FUNC_ALLOCA
AC_CHECK_HEADERS([
  fcntl.h
  netdb.h
  netinet/in.h
  sys/socket.h
  unistd.h
])


# ---------------------------------------------------------------------------- #

# Checks for typedefs, structures, and compiler characteristics.

AC_CHECK_HEADER_STDBOOL
AC_C_INLINE
AC_TYPE_INT64_T
AC_TYPE_SIZE_T
AC_TYPE_UINT64_T


# ---------------------------------------------------------------------------- #

# Checks for library functions.

AC_FUNC_ERROR_AT_LINE
AC_FUNC_LSTAT_FOLLOWS_SLASHED_SYMLINK
AC_HEADER_MAJOR
AC_CHECK_FUNCS([memset strtoul mkdir])


# ---------------------------------------------------------------------------- #

PKG_CHECK_MODULES([NLJSON], [nlohmann_json])
PKG_CHECK_MODULES([ARGPARSE], [argparse])
PKG_CHECK_MODULES([SQLITE3], [sqlite3])
PKG_CHECK_MODULES([SQLITE3PP], [sqlite3pp])
PKG_CHECK_MODULES([YAML], [yaml-cpp])
PKG_CHECK_MODULES([NIX], [nix-store nix-main nix-cmd nix-expr])


# ---------------------------------------------------------------------------- #

AC_CONFIG_FILES([config.mk])
AC_OUTPUT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
