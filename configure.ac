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
# TODO: read version from `.cz.toml'
AC_INIT([flox], [0.3.7], [alex.ameen@floxdev.com])

AC_CONFIG_MACRO_DIRS([build-aux/m4])
AC_CONFIG_AUX_DIR([build-aux])

AM_INIT_AUTOMAKE([-Wall -Werror foreign subdir-objects])

AC_CONFIG_SRCDIR([pkgdb/src/main.cc])
#AC_CONFIG_HEADERS([pkgdb/config.h])

LT_PREREQ([2.4.7])
LT_INIT

FLOX_INIT_DIRS


# ---------------------------------------------------------------------------- #

# Checks for programs.
# We use builtin `AC_PROG_<NAME>' macros to run checks; but follow up with
# `AC_PATH_PROG' to get absolute paths to the programs.

AC_PROG_CXX
AC_PATH_PROG([CXX], [$CXX], [$MISSING c++])

AC_PROG_CC
AC_PATH_PROG([CC], [$CC], [$MISSING cc])

AC_PROG_INSTALL
AC_PATH_PROG([CC], [$CC], [$MISSING install])

AC_PROG_GREP
AC_PATH_PROG([GREP], [$GREP], [$MISSING grep])

AC_PROG_SED
AC_PATH_PROG([SED], [$SED], [$MISSING sed])

AC_PROG_RANLIB
AC_PATH_PROG([RANLIB], [$RANLIB], [$MISSING ranlib])

AM_PROG_AR
AC_PATH_PROG([AR], [$AR], [$MISSING ar])


FLOX_PROG_MISSING

AC_PATH_PROG([CAT], [cat], [$MISSING cat])
AC_PATH_PROG([CP], [cp], [$MISSING cp])
AC_PATH_PROG([MKDIR], [mkdir], [$MISSING mkdir])
AC_PATH_PROG([RM], [rm -f], [$MISSING rm])
AC_PATH_PROG([SORT], [sort], [$MISSING sort])
AC_PATH_PROG([TEE], [tee], [$MISSING tee])
AC_PATH_PROG([TEST], [test], [$MISSING test])
AC_PATH_PROG([TOUCH], [touch], [$MISSING touch])
AC_PATH_PROG([TR], [tr], [$MISSING tr])
AC_PATH_PROG([UNAME], [uname], [$MISSING uname])
AC_PATH_PROG([LN], [ln], [$MISSING ln])

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

# Platform and Toolchain Checks

AC_CANONICAL_HOST

AS_CASE(["$host_os"],
  [linux*],  [OS=linux; sharedLibExt=.so;],
  [darwin*], [OS=darwin; sharedLibExt=.dylib;],
  [AC_MSG_ERROR([Unsupported host OS: $host_os])]
)
AC_SUBST([OS])
AC_SUBST([sharedLibExt])
AM_CONDITIONAL([IS_LINUX],   [test "$OS" = 'linux'])
AM_CONDITIONAL([IS_DARWIN],  [test "$OS" = 'darwin'])

AS_CASE(["$host_cpu"],
  [x86_64],   [ARCH=x86_64],
  [aarch64*], [ARCH=aarch64],
  [arm*],     [ARCH=aarch64],
  [AC_MSG_ERROR([Unsupported host ARCH: $host_cpu])]
)
AC_SUBST([ARCH])
AM_CONDITIONAL([IS_X86_64],  [test "$ARCH" = 'x86_64'])
AM_CONDITIONAL([IS_AARCH64], [test "$ARCH" = 'aarch64'])

AS_CASE(["$( $CXX --version; )"],
 [g++\ *],            [CXX_TOOLCHAIN=gcc],
 [clang\ version\ *], [CXX_TOOLCHAIN=clang],
 [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])]
)
AC_SUBST([CXX_TOOLCHAIN])
AM_CONDITIONAL([IS_GCC],   [test "$CXX_TOOLCHAIN" = 'gcc'])
AM_CONDITIONAL([IS_CLANG], [test "$CXX_TOOLCHAIN" = 'clang'])


# ---------------------------------------------------------------------------- #

# Checks for libraries.

AC_CHECK_LIB([nixfetchers],
  translit( [_ZN3nix8fetchers11attrsToJSONERKSt3mapINSt7]
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

# Directories

AC_SUBST([REPO_ROOT], ["$abs_srcdir"])
AC_SUBST([PKGDB_ROOT], ["$REPO_ROOT/pkgdb"])
AC_SUBST([PKGDB_TESTS_DIR], ["$PKGDB_ROOT/tests"])
AC_SUBST([ENV_BUILDER_TESTS_DIR], ["$ENV_BUILDER_ROOT/tests"])
AC_SUBST([FLOX_CLI_ROOT], ["$REPO_ROOT"])
AC_SUBST([FLOX_CLI_TESTS_DIR], ["$FLOX_CLI_ROOT/tests"])
AC_SUBST([CRATES_DIR], ["$FLOX_CLI_ROOT/crates"])
AC_SUBST([PKGS_DIR], ["$REPO_ROOT/pkgs"])
AC_SUBST([SHELLS_DIR], ["$REPO_ROOT/shells"])

# Used to determine if a dependency has changed.
DEPFILES="$REPO_ROOT/flake.nix $REPO_ROOT/flake.lock"
DEPFILES="$DEPFILES $PKGS_DIR/flox/default.nix";
DEPFILES="$DEPFILES $PKGS_DIR/flox-pkgdb/default.nix";
DEPFILES="$DEPFILES $SHELLS_DIR/flox/default.nix";
AC_SUBST([DEPFILES])


# ---------------------------------------------------------------------------- #

# External Dependencies

PKG_CHECK_MODULES([NLJSON], [nlohmann_json])
PKG_CHECK_MODULES([ARGPARSE], [argparse])
PKG_CHECK_MODULES([SQLITE3], [sqlite3])
PKG_CHECK_MODULES([SQLITE3PP], [sqlite3pp])
PKG_CHECK_MODULES([YAML], [yaml-cpp])
PKG_CHECK_MODULES([NIX], [nix-store nix-main nix-cmd nix-expr])


# ---------------------------------------------------------------------------- #

# Add some necessary macro definitions for `nix' which normally appear
# in `<nix>/include/nix/config.h'.
NIX_CFLAGS="$NIX_CFLAGS -DSYSTEM='\"$ARCH-$OS\"'";


# ---------------------------------------------------------------------------- #

# `RPATH', `RUNPATH', and `SONAME' flags
# --------------------------------------
# - `RELATIVE_RPATH_FLAG' is used to provide a relative `R[UN]PATH' for
#   an executable.
#   This is suitable for `bin/', `libexec/', `lib/', and `tests/' directory
#   members referring to binaries in the same sub-system; but is NOT suitable
#   for linking against libraries in other sub-systems.
# - `set_rpath_option' is used to set an absolute `R[UN]PATH' for a library.
#   This form is suitable for referring to binaries in other sub-systems.
#   It is intended to be used as:
#     LDFLAGS += '$(set_rpath_option)$(foo_LIBDIR)'
# - `set_soname_option' is used to set `SONAME' ( or equivalent ) for a library.
#   It is intended to be used as:
#     LDFLAGS += '$(set_soname_option)libfoo.$(sharedLibExt)'

AS_CASE(["$host_os"],
  [linux*],dnl
  [RELATIVE_RPATH_FLAG="-Wl,--enable-new-dtags";
  RELATIVE_RPATH_FLAG="$RELATIVE_RPATH_FLAG '-Wl,-rpath=\$\$\$\$ORIGIN/../lib'";
  set_rpath_option='-Wl,--enable-new-dtags,-rpath,';
  set_soname_option='-Wl,-soname,';],
  [darwin*],dnl
  [RELATIVE_RPATH_FLAG="-rpath @executable_path/../lib";
  set_rpath_option='-rpath ';
  set_soname_option='-install_name @rpath/';],
  [AC_MSG_ERROR([Unsupported host OS: $host_os])]
)

AC_SUBST([RELATIVE_RPATH_FLAG])
AC_SUBST([set_rpath_option])
AC_SUBST([set_soname_option])


# ---------------------------------------------------------------------------- #

# Shared Library Flags
# --------------------
# - `SHARED_LIBRARY_LDFLAGS' is used to link shared libraries.
# - `SHARED_LIBRARY_CFLAGS' is used to compile shared libraries.
# - `NO_UNDEFINED_SYMBOLS_FLAG' disallows undefined symbols in shared libraries.

AS_CASE(["$CXX_TOOLCHAIN"],
  [gcc],
  [NO_UNDEFINED_SYMBOLS_FLAG='-Wl,--no-undefined';
  SHARED_LIBRARY_CFLAGS='-fPIC -shared';],
  [clang],
  [NO_UNDEFINED_SYMBOLS_FLAG='-Wl,-undefined,error';
  SHARED_LIBRARY_CFLAGS='-fPIC';],
  [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])]
)

AC_SUBST([NO_UNDEFINED_SYMBOLS_FLAG])

SHARED_LIBRARY_LDFLAGS="$NO_UNDEFINED_SYMBOLS_FLAG $RELATIVE_RPATH_FLAG";
SHARED_LIBRARY_LDFLAGS="$SHARED_LIBRARY_LDFLAGS -fPIC -shared";
AC_SUBST([SHARED_LIBRARY_LDFLAGS])

AC_SUBST([SHARED_LIBRARY_CFLAGS])

AM_CONDITIONAL([BUILD_SHARED_LIBS], [test "$enable_shared" = 'yes'])


# ---------------------------------------------------------------------------- #

AC_SUBST([PKGDB_VERSION], ["$(< pkgdb/version)"])


# ---------------------------------------------------------------------------- #

AC_CONFIG_FILES([
  pkgdb/Makefile
])
AC_OUTPUT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
