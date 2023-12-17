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
AC_INIT([flox], [0.3.7])

AC_CONFIG_MACRO_DIRS([build-aux/m4])
AC_CONFIG_AUX_DIR([build-aux])

AM_INIT_AUTOMAKE([-Wall -Wno-portability -Werror foreign subdir-objects])

AC_CONFIG_SRCDIR([pkgdb/src/main.cc])

LT_PREREQ([2.4.7])
# NOTE: There's a bug in `libtool' that causes incorrect warnings to gripe
#       about `AM_PROG_AR' being called before `LT_INIT'.
#       The warnings are benign!
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

AC_PROG_CPP
AC_PATH_PROG([CPP], [$CPP], [$MISSING cpp])

AC_PROG_INSTALL
AC_PATH_PROG([CC], [$CC], [$MISSING install])

AC_PROG_GREP
AC_PATH_PROG([GREP], [$GREP], [$MISSING grep])

AC_PROG_SED
AC_PATH_PROG([SED], [$SED], [$MISSING sed])

# NOTE: `AC_PROG_RANLIB' is run by `LT_INIT'
#AC_PROG_RANLIB
AC_PATH_PROG([RANLIB], [$RANLIB], [$MISSING ranlib])

AM_PROG_AR
AC_PATH_PROG([AR], [$AR], [$MISSING ar])

PKG_PROG_PKG_CONFIG
AC_PATH_PROG([PKG_CONFIG], [$PKG_CONFIG], [$MISSING pkg-config])


FLOX_PROG_MISSING

AC_ARG_VAR([CAT], [Concatenate FILE(s) to standard output])
AC_PATH_PROG([CAT], [cat], [$MISSING cat])

AC_ARG_VAR([CP], [Copy files])
AC_PATH_PROG([CP], [cp], [$MISSING cp])

AC_ARG_VAR([MKDIR], [Create directories])
AC_PATH_PROG([MKDIR], [mkdir], [$MISSING mkdir])

AC_ARG_VAR([RM], [Remove files])
AC_PATH_PROG([RM], [rm -f], [$MISSING rm])

AC_ARG_VAR([SORT], [Sort lines of text])
AC_PATH_PROG([SORT], [sort], [$MISSING sort])

AC_ARG_VAR([TEE], [Redirect output to multiple files])
AC_PATH_PROG([TEE], [tee], [$MISSING tee])

AC_ARG_VAR([TEST], [Evaluate conditional expression])
AC_PATH_PROG([TEST], [test], [$MISSING test])

AC_ARG_VAR([TOUCH], [Change file timestamps, or create empty files])
AC_PATH_PROG([TOUCH], [touch], [$MISSING touch])

AC_ARG_VAR([TR],
  [Translate, squeeze, and/or delete characters from standard input])
AC_PATH_PROG([TR], [tr], [$MISSING tr])

AC_ARG_VAR([UNAME], [Print system information])
AC_PATH_PROG([UNAME], [uname], [$MISSING uname])

AC_ARG_VAR([LN], [Create symbolic links])
AC_PATH_PROG([LN], [ln], [$MISSING ln])

AC_ARG_VAR([FIND], [Search for files in a directory hierarchy])
AC_PATH_PROG([FIND], [find], [$MISSING find])

AC_ARG_VAR([XARGS], [Build and execute command lines from standard input])
AC_PATH_PROG([XARGS], [xargs], [$MISSING xargs])

AC_ARG_VAR([M4], [Macro processor])
AC_PATH_PROG([M4], [m4], [$MISSING m4])

AC_ARG_VAR([GIT], [Distributed version control system])
AC_PATH_PROG([GIT], [git], [$MISSING git])

AC_ARG_VAR([COMMITIZEN], [Conventional commit helper])
AC_PATH_PROG([COMMITIZEN], [commitizen], [$MISSING commitizen])

AC_ARG_VAR([HIVEMIND], [Process manager])
AC_PATH_PROG([HIVEMIND], [hivemind], [$MISSING hivemind])

AC_ARG_VAR([SQLITE], [Command line interface for SQLite])
AC_PATH_PROG([SQLITE], [sqlite3], [$MISSING sqlite3])

AC_ARG_VAR([BATS], [Bash Automated Testing System])
AC_PATH_PROG([BATS], [bats], [$MISSING bats])

# TODO: move to `m4/nix.m4'
AC_ARG_VAR([NIX], [Purely functional package manager])
AC_PATH_PROG([NIX], [nix], [$MISSING nix])
AC_ARG_VAR([NIX_FMT], [Nix code formatter])
AC_PATH_PROG([NIX_FMT], [alejandra], [$MISSING alejandra])

AC_ARG_VAR([CARGO], [Rust package manager])
AC_PATH_PROG([CARGO], [cargo], [$MISSING cargo])
AC_ARG_VAR([RUST_FMT], [Rust code formatter])
AC_PATH_PROG([RUST_FMT], [rustfmt], [$MISSING rustfmt])
AC_ARG_VAR([RUST_LINT], [Rust code linter])
AC_PATH_PROG([RUST_LINT], [clippy], [$MISSING clippy])

AC_ARG_VAR([BEAR], [Build EAR])
AC_PATH_PROG([BEAR], [bear], [$MISSING bear])

AC_ARG_VAR([CXX_FMT], [C++ code formatter])
AC_PATH_PROG([CXX_FMT], [clang-format], [$MISSING clang-format])

AC_ARG_VAR([CXX_LINT], [C++ code linter])
AC_PATH_PROG([CXX_LINT], [clang-tidy], [$MISSING clang-tidy])

AC_ARG_VAR([DOXYGEN], [Generate documentation])
AC_PATH_PROG([DOXYGEN], [doxygen], [$MISSING doxygen])

AC_ARG_VAR([SEMVER], [Semantic versioning helper])
AC_PATH_PROG([SEMVER], [semver], [$MISSING semver])

AC_ARG_VAR([TAR], [Manipulate tape archives])
AC_PATH_PROG([TAR], [tar], [$MISSING tar])

AC_ARG_VAR([RSYNC], [Remote file copy])
AC_PATH_PROG([RSYNC], [rsync], [$MISSING rsync])


# ---------------------------------------------------------------------------- #

# Platform and Toolchain Checks

AC_CANONICAL_HOST

AS_CASE(["$host_os"],
  [linux*],  [OS=linux; sharedLibExt=.so;],
  [darwin*], [OS=darwin; sharedLibExt=.dylib;],
  [AC_MSG_ERROR([Unsupported host OS: $host_os])])
AC_SUBST([OS])
AC_SUBST([sharedLibExt])
AM_CONDITIONAL([IS_LINUX],   [test "$OS" = 'linux'])
AM_CONDITIONAL([IS_DARWIN],  [test "$OS" = 'darwin'])

AS_CASE(["$host_cpu"],
  [x86_64],   [ARCH=x86_64],
  [aarch64*], [ARCH=aarch64],
  [arm*],     [ARCH=aarch64],
  [AC_MSG_ERROR([Unsupported host ARCH: $host_cpu])])
AC_SUBST([ARCH])
AM_CONDITIONAL([IS_X86_64],  [test "$ARCH" = 'x86_64'])
AM_CONDITIONAL([IS_AARCH64], [test "$ARCH" = 'aarch64'])

AS_CASE(["$( $CXX --version; )"],
 [g++\ *],            [CXX_TOOLCHAIN=gcc],
 [clang\ version\ *], [CXX_TOOLCHAIN=clang],
 [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])])
AC_SUBST([CXX_TOOLCHAIN])
AM_CONDITIONAL([IS_GCC],   [test "$CXX_TOOLCHAIN" = 'gcc'])
AM_CONDITIONAL([IS_CLANG], [test "$CXX_TOOLCHAIN" = 'clang'])


# ---------------------------------------------------------------------------- #

# Checks for libraries.

AC_CHECK_LIB([nixfetchers],
  translit( [_ZN3nix8fetchers11attrsToJSONERKSt3mapINSt7]
            [__cxx1112basic_stringIcSt11char_traitsIcESaIcEEESt7variantIJS7]
            [_mNS_8ExplicitIbEEEESt4lessIS7_ESaISt4pairIKS7_SB_EEE]
          , ' '))


# ---------------------------------------------------------------------------- #

# Checks for header files.

AC_FUNC_ALLOCA
AC_CHECK_HEADERS([
  fcntl.h
  netdb.h
  netinet/in.h
  sys/socket.h
  unistd.h
  utime.h
])


# ---------------------------------------------------------------------------- #

# Checks for typedefs, structures, and compiler characteristics.

AC_CHECK_HEADER_STDBOOL
AC_C_INLINE
AC_TYPE_SIZE_T
AC_TYPE_UINT8_T
AC_TYPE_INT64_T
AC_TYPE_UINT64_T


# ---------------------------------------------------------------------------- #

# Checks for library functions.

AC_FUNC_ERROR_AT_LINE
AC_FUNC_LSTAT_FOLLOWS_SLASHED_SYMLINK
AC_HEADER_MAJOR
AC_CHECK_FUNCS([memset strtoul mkdir utime])


# ---------------------------------------------------------------------------- #

# Sub-Systems/Modules

# --with-pkgdb
AC_ARG_WITH([pkgdb],
  [AS_HELP_STRING(
    [--with-pkgdb={local,nix,PREFIX}],
    [use pkgdb from local tree, nix build, or path to its install prefix. []dnl
( default: local )])],
  [WITH_PKGDB="$withval"],
  [WITH_PKGDB=local])
AS_CASE(["$WITH_PKGDB"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_PKGDB"], [:],
    [AC_MSG_ERROR([Invalid '--with-pkgdb': '$WITH_PKGDB'.
You must specify either 'local', 'nix', or a path to a directory])])])
AC_SUBST([WITH_PKGDB])

# --with-flox-cli
AC_ARG_WITH([flox-cli],
  [AS_HELP_STRING(
    [--with-flox-cli={local,nix,PREFIX}],
    [use flox-cli from local tree, nix build, or path to its ]dnl
  [install prefix. ( default: local )])],
    [WITH_FLOX_CLI="$withval"],
    [WITH_FLOX_CLI=local])
AS_CASE(["$WITH_FLOX_CLI"],
  [local], [:],
  [nix],   [:],
  [AS_IF([test -d "$WITH_FLOX_CLI"], [:],
    [AC_MSG_ERROR([Invalid '--with-flox-cli': '$WITH_FLOX_CLI'.
You must specify either 'local', 'nix', or a path to a directory])])])
AC_SUBST([WITH_FLOX_CLI])


# ---------------------------------------------------------------------------- #

# Directories

AC_SUBST([REPO_ROOT], ["$abs_srcdir"])
AC_SUBST([PKGDB_ROOT], ["$REPO_ROOT/pkgdb"])
AC_SUBST([PKGDB_TESTS_DIR], ["$PKGDB_ROOT/tests"])
AC_SUBST([FLOX_CLI_ROOT], ["$REPO_ROOT/cli"])
AC_SUBST([FLOX_CLI_TESTS_DIR], ["$FLOX_CLI_ROOT/tests"])
AC_SUBST([FLOX_CLI_DISTDIR], ["$FLOX_CLI_ROOT/dist"])
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

PKG_CHECK_MODULES([NLJSON], [nlohmann_json], [:],
  [AC_MSG_ERROR([Cannot find 'nlohmann_json.pc'])])

PKG_CHECK_MODULES([ARGPARSE], [argparse], [:],
  [AC_MSG_ERROR([Cannot find 'argparse.pc'])])

PKG_CHECK_MODULES([SQLITE3], [sqlite3], [:],
  [AC_MSG_ERROR([Cannot find 'sqlite3.pc'])])

PKG_CHECK_MODULES([SQLITE3PP], [sqlite3pp], [:],
  [AC_MSG_ERROR([Cannot find 'sqlite3pp.pc'])])

PKG_CHECK_MODULES([YAML], [yaml-cpp], [:],
  [AC_MSG_ERROR([Cannot find 'yaml-cpp.pc'])])

PKG_CHECK_MODULES([NIX], [nix-store nix-main nix-cmd nix-expr], [:],
  [AC_MSG_ERROR([Cannot find 'nix-{store,main,cmd,expr}.pc'])])
NIX_LIBS="-lnixfetchers $NIX_LIBS"


# ---------------------------------------------------------------------------- #

# Compiler Flags for pre-processing with `nix' headers

AC_MSG_CHECKING([Nix system pair])
AC_ARG_VAR([SYSTEM], [Nix system pair])
SYSTEM="$OS-$ARCH";
AC_MSG_RESULT([$SYSTEM])
AC_SUBST([SYSTEM])

AC_DEFINE_UNQUOTED([SYSTEM], ["$SYSTEM"], [Nix system pair])


# Check configuration used to compile `nix' and add it to `NIX_CPPFLAGS`.

_flox_v_pushed_CPPFLAGS="$CPPFLAGS";
CPPFLAGS="$NIX_CPPFLAGS";
AC_LANG_PUSH([C++])

# NOTE: "@%:@" is replaced with "#" by `autoconf' to prevent `m4sh' from
#       treating it as a comment.
AC_MSG_CHECKING([Nix configuration setting HAVE_BOEHMGC])
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef HAVE_BOEHMGC
     @%:@error "Boehm GC support is not indicated"
   @%:@endif
   @%:@if HAVE_BOEHMGC != 1
     @%:@error "Boehm GC is not supported"
   @%:@endif])],
  [AC_DEFINE_UNQUOTED([HAVE_BOEHMGC])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])

AC_MSG_CHECKING([Nix configuration setting ENABLE_S3])
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef ENABLE_S3
     @%:@error "S3 is not indicated"
   @%:@endif
   @%:@if ENABLE_S3 != 1
     @%:@error "S3 is not supported"
   @%:@endif])],
  [AC_DEFINE_UNQUOTED([ENABLE_S3])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])

AC_MSG_CHECKING([Nix configuration setting HAVE_STRUCT_DIRENT_D_TYPE])
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef HAVE_STRUCT_DIRENT_D_TYPE
     @%:@error "HAVE_STRUCT_DIRENT_D_TYPE is not defined"
   @%:@endif
   @%:@if HAVE_STRUCT_DIRENT_D_TYPE != 1
     @%:@error "HAVE_STRUCT_DIRENT_D_TYPE is false"
   @%:@endif])],
  [AC_DEFINE([HAVE_STRUCT_DIRENT_D_TYPE])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])

AC_MSG_CHECKING([Nix configuration setting EXCEPTION_NEEDS_THROW_SPEC])
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef EXCEPTION_NEEDS_THROW_SPEC
     @%:@error "EXCEPTION_NEEDS_THROW_SPEC is not defined"
   @%:@endif
   @%:@if EXCEPTION_NEEDS_THROW_SPEC != 1
     @%:@error "EXCEPTION_NEEDS_THROW_SPEC is false"
   @%:@endif])],
  [AC_DEFINE([EXCEPTION_NEEDS_THROW_SPEC])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])

# `HAVE_BOOST' does NOT use an integer value in `<nix/config.h>'.
# It is either defined or not ( they use `/**/' on the right hand side ).
# We use a `1' - I don't think this is a problem but wanted to note it.
AC_MSG_CHECKING([Nix configuration setting HAVE_BOOST])
AC_PREPROC_IFELSE([AC_LANG_SOURCE(
  [@%:@include <nix/config.h>
   @%:@ifndef HAVE_BOOST
     @%:@error "HAVE_BOOST is not defined"
   @%:@endif])],
  [AC_DEFINE([HAVE_BOOST])
   AC_MSG_RESULT([yes])],
  [AC_MSG_RESULT([no])])

AC_LANG_POP
CPPFLAGS="$_flox_v_pushed_CPPFLAGS";

# NOTE: About `<nix/config.h>' and our configuration macros:
# When upgrading `nix' it's possible that new macros will be added or files
# sensitivity to certain macros may change.
# If you encounter issues, you can uncomment the following lines to try using
# the _upstream_ `<nix/config.h>' instead of our own subset.
# This is expected to emit warnings about redefined macros; but it will at least
# help you isolate the problem.
#NIX_INCDIR
#PKG_CHECK_VAR([NIX_INCDIR], [nix-main], [includedir],
#  [NIX_CFLAGS="-isystem $NIX_INCDIR"],
#  [AC_MSG_ERROR([Cannot find nix-main include directory for 'nix/config.h'])])
#NIX_CFLAGS="$NIX_CFLAGS -include $NIX_INCDIR/nix/config.h"
#AC_SUBST([NIX_CFLAGS])


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
    RELATIVE_RPATH_FLAG="$RELATIVE_RPATH_FLAG []dnl
'-Wl,-rpath=\$\$ORIGIN/../lib'";
    set_rpath_option='-Wl,--enable-new-dtags,-rpath,';
    set_soname_option='-Wl,-soname,';],
  [darwin*],dnl
    [RELATIVE_RPATH_FLAG="-rpath @executable_path/../lib";
    set_rpath_option='-rpath ';
    set_soname_option='-install_name @rpath/';],
  [AC_MSG_ERROR([Unsupported host OS: $host_os])])

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
  [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])])

AC_SUBST([NO_UNDEFINED_SYMBOLS_FLAG])

SHARED_LIBRARY_LDFLAGS="$NO_UNDEFINED_SYMBOLS_FLAG $RELATIVE_RPATH_FLAG";
SHARED_LIBRARY_LDFLAGS="$SHARED_LIBRARY_LDFLAGS -fPIC -shared";
AC_SUBST([SHARED_LIBRARY_LDFLAGS])

AC_SUBST([SHARED_LIBRARY_CFLAGS])

AM_CONDITIONAL([BUILD_SHARED_LIBS], [test "$enable_shared" = 'yes'])


# ---------------------------------------------------------------------------- #

# Get `pkgdb' version

AC_SUBST([PKGDB_VERSION], ["$(< $PKGDB_ROOT/.version)"])
AC_DEFINE_UNQUOTED([PKGDB_VERSION], ["$PKGDB_VERSION"], [Version of pkgdb])


# ---------------------------------------------------------------------------- #

# Get paths to static assets

AC_ARG_VAR([SET_PROMPT_BASH_SH],
  [Path to 'set-prompt.bash.sh' script in '/nix/store/'])

# --with-set-prompt-bash-sh
AC_ARG_WITH([set-prompt-bash-sh],
  [AS_HELP_STRING(
    [--with-set-prompt-bash-sh={local,PATH}],
    [use set-prompt-bash.sh from local tree, or a '/nix/store' path. []dnl
( default: local )])],
  [WITH_SET_PROMPT_BASH_SH="$withval"],
  [WITH_SET_PROMPT_BASH_SH=local])
AS_IF([test -z "${SET_PROMPT_BASH_SH:-}"], [
  AS_CASE(["$WITH_SET_PROMPT_BASH_SH"],
    [/nix/store/*], [SET_PROMPT_BASH_SH="$WITH_SET_PROMPT_BASH_SH"],
    [local],        [SET_PROMPT_BASH_SH="$(
      $NIX --extra-experimental-features='nix-command flakes'  \
           store add-path -n set-prompt-bash.sh                \
           "$PKGDB_ROOT/src/buildenv/set-prompt-bash.sh";
      )"],
    [AS_IF([test -d "$WITH_SET_PROMPT_BASH_SH"], [:],
      [AC_MSG_ERROR(
        [Invalid '--with-set-prompt-bash-sh': '$WITH_SET_PROMPT_BASH_SH'.
You must specify either 'local', or a '/nix/store' path])])])])
AC_SUBST([WITH_SET_PROMPT_BASH_SH])
AC_DEFINE_UNQUOTED([SET_PROMPT_BASH_SH], ["$SET_PROMPT_BASH_SH"],
  [Path to 'set-prompt.bash.sh' script in '/nix/store/'])


AC_ARG_VAR([PROFILE_D_SCRIPT_DIR],
  [Path to directory containing 'etc/profile.d' directory in '/nix/store/'])

# --with-profile-d-script-dir
AC_ARG_WITH([profile-d-script-dir],
  [AS_HELP_STRING(
    [--with-profile-d-script-dir={local,PATH}],
    [use 'etc/profile.d' from local tree, or a '/nix/store' path. []dnl
( default: local )])],
  [WITH_PROFILE_D_SCRIPT_DIR="$withval"],
  [WITH_PROFILE_D_SCRIPT_DIR=local])
AS_IF([test -z "${PROFILE_D_SCRIPT_DIR:-}"], [
  AS_CASE(["$WITH_PROFILE_D_SCRIPT_DIR"],
    [/nix/store/*], [PROFILE_D_SCRIPT_DIR="$WITH_PROFILE_D_SCRIPT_DIR"],
    [local],        [PROFILE_D_SCRIPT_DIR="$(
      $NIX --extra-experimental-features='nix-command flakes'  \
           store add-file -n set-prompt-bash.sh                \
           "$PKGDB_ROOT/src/buildenv/assets";
      )"],
    [AS_IF([test -d "$WITH_PROFILE_D_SCRIPT_DIR"], [:],
      [AC_MSG_ERROR(
        [Invalid '--with-profile-d-script-dir': '$WITH_PROFILE_D_SCRIPT_DIR'.
You must specify either 'local', or a '/nix/store' path])])])])
AC_SUBST([WITH_PROFILE_D_SCRIPT_DIR])
AC_DEFINE_UNQUOTED([PROFILE_D_SCRIPT_DIR], ["$PROFILE_D_SCRIPT_DIR"],
  [Path to directory containing 'etc/profile.d' directory in '/nix/store/'])


# ---------------------------------------------------------------------------- #

# Get path to `semver' in `/nix/store'

AC_ARG_VAR([SEMVER_PATH], [Path to 'semver' executable in '/nix/store/'])
# NOTE: `@%:@' is replaced with `#' by `autoconf' so that it is not treated as
#       a comment.
AS_CASE(["$SEMVER"],
  [/nix/store/*], [SEMVER_PATH="$SEMVER"],
  [SEMVER_PATH="$(
     $NIX --extra-experimental-features='nix-command flakes'  \
          build --no-link --print-out-paths                   \
          'github:aakropotkin/floco@%:@semver'
   )/bin/semver"])
AC_SUBST([SEMVER_PATH])
AC_DEFINE_UNQUOTED([SEMVER_PATH], ["$SEMVER_PATH"],
  [Path to 'semver' executable in '/nix/store'])


# ---------------------------------------------------------------------------- #

# Cargo flags

AC_ARG_VAR([CARGOFLAGS], [Additional flags to pass to 'cargo build'])
AC_SUBST([CARGOFLAGS])


# ---------------------------------------------------------------------------- #

AC_CONFIG_FILES([
  Makefile
  pkgdb/Makefile
  cli/Makefile
])
AC_OUTPUT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
