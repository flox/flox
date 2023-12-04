# -*- mode: autoconf -*-
# ============================================================================ #
#
# @file configure.ac
#
# @brief Process this file with autoconf to produce a configure script.
#
#
# ---------------------------------------------------------------------------- #

AC_PREREQ( [2.71] )

# TODO: use different email address
AC_INIT([flox], [0.3.7], [alex.ameen@floxdev.com])

AC_CONFIG_MACRO_DIRS([m4])
AC_CONFIG_AUX_DIR([build-aux])

AM_INIT_AUTOMAKE([-Wall -Werror foreign subdir-objects])

AC_CONFIG_SRCDIR([pkgdb/src/main.cc])
#AC_CONFIG_HEADERS([pkgdb/config.h])

# Absolute path to the directory containing `configure' script.
AC_SUBST([abs_srcdir], [$( cd "$srcdir"; echo "$PWD"; )])


# ---------------------------------------------------------------------------- #

# Checks for programs.

AC_PROG_CXX
AC_PROG_CC
AC_PROG_INSTALL
AC_PROG_LN_S
AC_PROG_GREP
AC_PROG_SED
AC_PROG_LN_S

AC_PATH_PROG([MISSING], [missing], [AC_MSG_ERROR([missing not found])],
             [$PATH:$abs_srcdir/build-aux])

AC_PATH_PROG([CAT], [cat], [$MISSING cat])
AC_PATH_PROG([CP], [cp], [$MISSING cp])
AC_PATH_PROG([MKDIR], [mkdir], [$MISSING mkdir])
AC_PATH_PROG([RM], [rm], [$MISSING rm])
AC_PATH_PROG([SORT], [sort], [$MISSING sort])
AC_PATH_PROG([TEE], [tee], [$MISSING tee])
AC_PATH_PROG([TEST], [test], [$MISSING test])
AC_PATH_PROG([TOUCH], [touch], [$MISSING touch])
AC_PATH_PROG([TR], [tr], [$MISSING tr])
AC_PATH_PROG([UNAME], [uname], [$MISSING uname])

AC_PATH_PROG([FIND], [find], [$MISSING find])
AC_PATH_PROG([XARGS], [xargs], [$MISSING xargs])

AC_PATH_PROG([M4], [m4], [$MISSING m4])

AC_PATH_PROG([GIT], [git], [$MISSING git])

AC_PATH_PROG([COMMITIZEN], [commitizen], [$MISSING commitizen])
AC_PATH_PROG([HIVEMIND], [hivemind], [$MISSING hivemind])
AC_PATH_PROG([SQLITE], [sqlite3], [$MISSING sqlite3])
AC_PATH_PROG([BATS], [bats], [$MISSING bats])

# TODO: move to `m4/nix.m4'
AC_PATH_PROG([NIX], [nix], [$MISSING nix])
AC_PATH_PROG([NIX_FMT], [alejandra], [$MISSING alejandra])

AC_PATH_PROG([CARGO], [cargo], [$MISSING cargo])
AC_PATH_PROG([RUST_FMT], [rustfmt], [$MISSING rustfmt])
AC_PATH_PROG([RUST_LINT], [clippy], [$MISSING clippy])

AC_PATH_PROG([BEAR], [bear], [$MISSING bear])
AC_PATH_PROG([CXX_FMT], [clang-format], [$MISSING clang-format])
AC_PATH_PROG([CXX_LINT], [clang-tidy], [$MISSING clang-tidy])
AC_PATH_PROG([DOXYGEN], [doxygen], [$MISSING doxygen])


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

# Sub-Systems/Modules

# --with-flox-bash
AC_ARG_WITH([flox-bash],
  [AS_HELP_STRING(
    [--with-flox-bash={local,nix,PREFIX}],
    [use flox-bash from local tree, nix build, or path to its install prefix])],
  [WITH_FLOX_BASH="$withval"],
  [WITH_FLOX_BASH=local]
)
AS_CASE(["$WITH_FLOX_BASH"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_FLOX_BASH"], [:],
    [AC_MSG_ERROR([Invalid WITH_FLOX_BASH setting: '$WITH_FLOX_BASH'.
You must specify either 'local', 'nix', or a path to a directory])])
])
AC_SUBST([WITH_FLOX_BASH])

# --with-pkgdb
AC_ARG_WITH([pkgdb],
  [AS_HELP_STRING(
    [--with-pkgdb={local,nix,PREFIX}],
    [use pkgdb from local tree, nix build, or path to its install prefix])],
  [WITH_PKGDB="$withval"],
  [WITH_PKGDB=local]
)
AS_CASE(["$WITH_PKGDB"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_PKGDB"], [:],
    [AC_MSG_ERROR([Invalid '--with-pkgdb' setting: '$WITH_PKGDB'.
You must specify either 'local', 'nix', or a path to a directory])])
])
AC_SUBST([WITH_PKGDB])

# --with-env-builder
AC_ARG_WITH([env-builder],
  [AS_HELP_STRING(
    [--with-env-builder={local,nix,PREFIX}],
    [use env-builder from local tree, nix build, or path to its ]dnl
[install prefix])],
  [WITH_ENV_BUILDER="$withval"],
  [WITH_ENV_BUILDER=local]
)
AS_CASE(["$WITH_ENV_BUILDER"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_ENV_BUILDER"], [:],
    [AC_MSG_ERROR([Invalid '--with-env-builder' setting: '$WITH_ENV_BUILDER'.
You must specify either 'local', 'nix', or a path to a directory])])
])
AC_SUBST([WITH_ENV_BUILDER])

# --with-flox-cli
AC_ARG_WITH([flox-cli],
  [AS_HELP_STRING(
    [--with-flox-cli={local,nix,PREFIX}],
    [use flox-cli from local tree, nix build, or path to its ]dnl
[install prefix])],
  [WITH_FLOX_CLI="$withval"],
  [WITH_FLOX_CLI=local]
)
AS_CASE(["$WITH_FLOX_CLI"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_FLOX_CLI"], [:],
    [AC_MSG_ERROR([Invalid '--with-flox-cli' setting: '$WITH_FLOX_CLI'.
You must specify either 'local', 'nix', or a path to a directory])])
])
AC_SUBST([WITH_FLOX_CLI])


# ---------------------------------------------------------------------------- #

# External Dependencies

PKG_CHECK_MODULES([NLJSON], [nlohmann_json])
PKG_CHECK_MODULES([ARGPARSE], [argparse])
PKG_CHECK_MODULES([SQLITE3], [sqlite3])
PKG_CHECK_MODULES([SQLITE3PP], [sqlite3pp])
PKG_CHECK_MODULES([YAML], [yaml-cpp])
PKG_CHECK_MODULES([NIX], [nix-store nix-main nix-cmd nix-expr])


# ---------------------------------------------------------------------------- #

AC_CONFIG_FILES([Makefile config.mk])
AC_OUTPUT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
