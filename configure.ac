# -*- mode: autoconf -*-
# ============================================================================ #
#
# @file configure.ac
#
# @brief Process this file with autoconf to produce a configure script.
#
# Macros rule.
#
# See the `autoconf' manual for nice documentation about various functions.
# You can read macro definitions in `./aclocal', `./build-aux/m4/', and
# `$( nix eval --raw nixpkgs#autoconf )/share/autoconf/"{autoconf/m4sugar}/*.m4'
#
# `configure' might look _scary_ at first; but always remember that it's just
# a glorified `sed' processor replaces `@FOO@' in `FILE.in' with the environment
# variable `FOO' ( derived while running `configure' or set by the caller ).
#
# `autoconf' is actually scary, but there's plenty of good docs, and it
# contains decades worth of platform specific handlers, portability checks,
# and robust recipes for common tasks that make it _well worth using_.
#
# ALL build systems are _bad_, especially at platform dependent tasks - they
# either throw their hands in the air and statically link everything, or they
# fail to translate various flags to the compiler/linker correctly and fail to
# account for the variety of platforms and toolchains' unique headers
# and configurations.
# `autotools' isn't great, but it's the best tool we have for portability,
# and its widespread use in open source projects provides a large body of
# existing knowledge, Stack Overflow threads, and examples to draw from.
#
#
# ---------------------------------------------------------------------------- #

# Initialize Autoconf
# -------------------

AC_PREREQ([2.71])

# TODO: read version from `.cz.toml'
AC_INIT([flox], [0.3.7])

AC_CONFIG_MACRO_DIRS([build-aux/m4])
AC_CONFIG_AUX_DIR([build-aux])

# - Enable all warnings, but allow GNU Make extensions.
# - Treat all warnings as errors.
# - Use `subdir-objects' so that build artifacts are placed in sub-directories.
# - Enable `foreign' mode so that warnings about strict adherence to
#   GNU Standards is suppressed ( ex: must set contact email, must
#   have up-to-date GPL LICENSE, must define certain targets, must support
#   `DESTDIR', etc ).
AM_INIT_AUTOMAKE([-Wall -Wno-portability -Werror subdir-objects foreign])

AC_CONFIG_SRCDIR([pkgdb/src/main.cc])

LT_PREREQ([2.4.7])
# NOTE: There's a bug in `libtool' that causes incorrect warnings to gripe
#       about `AM_PROG_AR' being called before `LT_INIT'.
#       The warnings are benign!
LT_INIT

# Defines `PREFIX', and similar ALL_CAPS standard directory variables.
# These are aliases of lower-case forms.
# Also defined `INSTALL_TESTS_DIR' and `PKGCONFIGDIR'.
# Sets `abs_srcdir' early so it can be used to define `REPO_ROOT' and other
# variables relative to the project root.
FLOX_INIT_DIRS


# ---------------------------------------------------------------------------- #

# Checks for Programs
# -------------------

FLOX_PROG_MISSING

# We use builtin `AC_PROG_<NAME>' macros to run checks; but follow up with
# `AC_PATH_PROG' to get absolute paths to the programs.

AC_PROG_CXX
AC_PATH_PROG([CXX], ["${CXX:-c++}"], [$MISSING c++])

# We use a modified `autoreconf' script that patches `configure' after
# creating it, and `build-aux/m4/libtool.m4' after `libtoolize' is run.
# We need to know the path to the original `autoreconf' script so that we can
# run it from inside that script.
AC_ARG_VAR([REAL_AUTORECONF], [Path to the original `autoreconf' script])
AC_PATH_PROG([REAL_AUTORECONF], [autoreconf], [$MISSING autoreconf])
AC_ARG_VAR([AUTORECONF],
  [Path to the script to use as `autoreconf' inside of Makefiles])
AC_PATH_PROG([AUTORECONF], [autoreconf], [$MISSING build-aux/autoreconf],
  [$abs_srcdir/build-aux/:$PATH])

FLOX_PROG_CC

AC_PROG_CPP
AC_PATH_PROG([CPP], ["${CPP:-cpp}"], [$MISSING cpp])

AC_PROG_INSTALL
AC_PATH_PROG([INSTALL], ["${INSTALL:-install}"], [$MISSING install])

FLOX_PROG_GREP

AC_PROG_SED
AC_PATH_PROG([SED], ["${SED:-sed}"], [$MISSING sed])

# NOTE: `AC_PROG_RANLIB' is run by `LT_INIT'
#AC_PROG_RANLIB
AC_PATH_PROG([RANLIB], ["${RANLIB:-ranlib}"], [$MISSING ranlib])

AM_PROG_AR
AC_PATH_PROG([AR], ["${AR:-ar}"], [$MISSING ar])

PKG_PROG_PKG_CONFIG
AC_PATH_PROG([PKG_CONFIG], ["${PKG_CONFIG:-}"], [$MISSING pkg-config])

AC_ARG_VAR([CAT], [Concatenate FILE(s) to standard output])
AC_PATH_PROG([CAT], [cat], [$MISSING cat])

AC_ARG_VAR([HEAD], [Output the first part of files])
AC_PATH_PROG([HEAD], [head], [$MISSING head])

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

# Sets `NIX'
FLOX_PROG_NIX
# Flags passed to `nix' invocations.
AC_ARG_VAR([NIXFLAGS],
  [Flags passed as 'nix <NIXFLAGS> <CMD> ...' to all 'nix' invocations ]
  [@<:@default=--experimental-features='nix-command flakes'@:>@])
AC_SUBST([NIXFLAGS], ["--experimental-features='nix-command flakes'"])

AC_ARG_VAR([NIX_FMT], [Nix code formatter @<:@default=alejandra@:>@])
AC_PATH_PROG([NIX_FMT], [alejandra], [$MISSING alejandra])

AC_ARG_VAR([CARGO], [Rust package manager])
AC_PATH_PROG([CARGO], [cargo], [$MISSING cargo])
AC_ARG_VAR([RUST_FMT], [Rust code formatter @<:@default=rustfmt@:>@])
AC_PATH_PROG([RUST_FMT], [rustfmt], [$MISSING rustfmt])
AC_ARG_VAR([RUST_LINT], [Rust code linter @<:@default=clippy@:>@])
AC_PATH_PROG([RUST_LINT], [clippy], [$MISSING clippy])

AC_ARG_VAR([BEAR], [Build EAR])
AC_PATH_PROG([BEAR], [bear], [$MISSING bear])

AC_ARG_VAR([CXX_FMT], [C++ code formatter @<:@default=clang-format@:>@])
AC_PATH_PROG([CXX_FMT], [clang-format], [$MISSING clang-format])

AC_ARG_VAR([CXX_LINT], [C++ code linter @<:@default=clang-tidy@:>@])
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
# -----------------------------

AC_CANONICAL_HOST

AS_CASE(["$host_os"],
  [linux*],  [OS=linux],
  [darwin*], [OS=darwin],
  [AC_MSG_ERROR([Unsupported host OS: $host_os])])
AC_SUBST([OS])
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

AC_CACHE_CHECK([for C++ compiler toolchain], [ac_cv_cxx_toolchain],
  [AS_CASE(["$( $CXX --version|$HEAD -n1; )"],
     [g++\ *],            [ac_cv_cxx_toolchain=gcc],
     [clang\ version\ *], [ac_cv_cxx_toolchain=clang],
     [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])])
   AC_MSG_RESULT([$ac_cv_cxx_toolchain])])
AC_SUBST([CXX_TOOLCHAIN])
AM_CONDITIONAL([IS_GCC],   [test "$CXX_TOOLCHAIN" = 'gcc'])
AM_CONDITIONAL([IS_CLANG], [test "$CXX_TOOLCHAIN" = 'clang'])


# ---------------------------------------------------------------------------- #

# Checks for Libraries
# --------------------
FLOX_LIB_NIXFETCHERS


# ---------------------------------------------------------------------------- #

# Checks for Header Files
# -----------------------

AC_HEADER_MAJOR
AC_CHECK_HEADERS([
  fcntl.h
  netdb.h
  netinet/in.h
  sys/socket.h
  unistd.h
  utime.h
])


# ---------------------------------------------------------------------------- #

# Checks for typedefs, structures, and compiler characteristics
# -------------------------------------------------------------

AC_CHECK_HEADER_STDBOOL
AC_C_INLINE
AC_TYPE_SIZE_T
AC_TYPE_UINT8_T
AC_TYPE_INT64_T
AC_TYPE_UINT64_T


# ---------------------------------------------------------------------------- #

# Checks for Library Functions
# ----------------------------

AC_FUNC_ALLOCA
AC_FUNC_ERROR_AT_LINE
AC_FUNC_LSTAT_FOLLOWS_SLASHED_SYMLINK
AC_CHECK_FUNCS([memset strtoul mkdir utime])


# ---------------------------------------------------------------------------- #

# Cargo Flags
# -----------

AC_ARG_VAR([CARGOFLAGS], [Additional flags to pass to 'cargo build'])
AC_SUBST([CARGOFLAGS])


# ---------------------------------------------------------------------------- #

# Debug Mode
# ----------

AS_IF([test "$CXX_TOOLCHAIN" = 'gcc'],
  [DEBUG_CXXFLAGS='-ggdb3 -pg';
   DEBUG_LDFLAGS='-ggdb3 -pg';],
  [DEBUG_CXXFLAGS='-g -glldb -mno-omit-leaf-frame-pointer';
   DEBUG_LDFLAGS='-g -glldb -mno-omit-leaf-frame-pointer';])
DEBUG_CXXFLAGS="$DEBUG_CXXFLAGS -fno-omit-frame-pointer -DDEBUG";
DEBUG_LDFLAGS="$DEBUG_CXXFLAGS -fno-omit-frame-pointer -DDEBUG";
AC_SUBST([DEBUG_CXXFLAGS])

AC_ARG_ENABLE([debug],
  [AS_HELP_STRING([--enable-debug], [enable debug mode @<:@default=no@:>@])],
  [ENABLE_DEBUG="$enableval"],
  [ENABLE_DEBUG=no])
AM_CONDITIONAL([ENABLE_DEBUG], [test "$ENABLE_DEBUG" = 'yes'])

# Translate `0' -> `no' in case the environment variable is was used.
AS_IF([test "$ENABLE_DEBUG" = 0], [ENABLE_DEBUG=no])

AS_IF([test "$ENABLE_DEBUG" != 'no'],
  [AC_DEFINE_UNQUOTED([DEBUG], [1], [Enable debug mode])
   CXXFLAGS="$CXXFLAGS $DEBUG_CXXFLAGS";
   LDFLAGS="$LDFLAGS $DEBUG_LDFLAGS";
   AC_MSG_NOTICE([Enabled debug mode])])


# ---------------------------------------------------------------------------- #

# Coverage Mode
# -------------

COV_CXXFLAGS='--coverage -fprofile-arcs -ftest-coverage';
COV_LDFLAGS='--coverage -fprofile-arcs -ftest-coverage';
AC_SUBST([COV_CXXFLAGS])

AC_ARG_ENABLE([coverage],
  [AS_HELP_STRING([--enable-coverage],
                  [enable coverage mode @<:@default=no@:>@])],
  [ENABLE_COV="$enableval"],
  [ENABLE_COV=no])
AM_CONDITIONAL([ENABLE_COV], [test "$ENABLE_COV" = 'yes'])

# Translate `0' -> `no' in case the environment variable is was used.
AS_IF([test "$ENABLE_COV" = 0], [ENABLE_COV=no])

AS_IF([test "$ENABLE_COV" != 'no'],
  [CXXFLAGS="$CXXFLAGS $COV_CXXFLAGS";
   LDFLAGS="$LDFLAGS $COV_LDFLAGS";
   AC_MSG_NOTICE([Enabled coverage mode])])


# ---------------------------------------------------------------------------- #

# Release Mode
# ------------

RELEASE_CXXFLAGS='-O3 -DNDEBUG';
RELEASE_LDFLAGS='-O3';
AC_SUBST([RELEASE_CXXFLAGS])

AC_ARG_ENABLE([release],
  [AS_HELP_STRING([--enable-release],
                  [enable release mode @<:@default=no@:>@])],
  [ENABLE_RELEASE="$enableval"],
  [ENABLE_RELEASE=no])
AM_CONDITIONAL([ENABLE_RELEASE], [test "$ENABLE_RELEASE" = 'yes'])

# Translate `0' -> `no' in case the environment variable is was used.
AS_IF([test "$ENABLE_RELEASE" = 0], [ENABLE_RELEASE=no])

AS_IF([test "$ENABLE_RELEASE" != 'no'],
  [AS_IF([test "$ENABLE_RELEASE" != 'no'],
         [AC_MSG_ERROR([Release mode and debug mode may not be used together])])
   AC_DEFINE_UNQUOTED([RELEASE], [1], [Enable release mode])
   CXXFLAGS="$CXXFLAGS $RELEASE_CXXFLAGS";
   LDFLAGS="$LDFLAGS $RELEASE_LDFLAGS";
   CARGOFLAGS="${CARGOFLAGS:+$CARGOFLAGS }--release";
   FLOX_CLI_targetdir='target/release';
   AC_MSG_NOTICE([Enabled release mode])])


# ---------------------------------------------------------------------------- #

# Sub-Systems/Modules Options
# ---------------------------

# --with-pkgdb
AC_ARG_WITH([pkgdb],
  [AS_HELP_STRING(
    [--with-pkgdb=local|nix|PREFIX],
    [use pkgdb from local tree, nix build, or path to its install prefix ]
    [@<:@default=local@:>@])],
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
    [--with-flox-cli=local|nix|PREFIX],
    [use flox-cli from local tree, nix build, or path to its ]
  [install prefix @<:@default=local@:>@])],
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

# Project Directories
# -------------------

AC_SUBST([REPO_ROOT], ["$abs_srcdir"])
AC_SUBST([PKGS_DIR], ["$REPO_ROOT/pkgs"])
AC_SUBST([SHELLS_DIR], ["$REPO_ROOT/shells"])
AC_SUBST([PKGDB_ROOT], ["$REPO_ROOT/pkgdb"])
AC_SUBST([PKGDB_TEST_DIR], ["$PKGDB_ROOT/tests"])
AC_SUBST([FLOX_CLI_ROOT], ["$REPO_ROOT/cli"])
AC_SUBST([FLOX_CLI_TESTS_DIR], ["$FLOX_CLI_ROOT/tests"])

# Defined relative to `$(abs_top_builddir)/cli/'
AC_SUBST([FLOX_CLI_distdir],   [dist])
AC_SUBST([FLOX_CLI_targetdir], [target/debug])
AC_SUBST([FLOX_CLI_tardir],    ["$distdir"])

# Used to determine if a dependency has changed.
DEPFILES="$REPO_ROOT/flake.nix $REPO_ROOT/flake.lock"
DEPFILES="$DEPFILES $PKGS_DIR/flox/default.nix";
DEPFILES="$DEPFILES $PKGS_DIR/flox-pkgdb/default.nix";
DEPFILES="$DEPFILES $SHELLS_DIR/flox/default.nix";
AC_SUBST([DEPFILES])


# ---------------------------------------------------------------------------- #

# External Dependencies
# ---------------------
# Each `PKG_CHECK_MODULES([PKG], [PC-NAME])' invocation sets `<PKG>_CFLAGS' and
# `<PKG>_LIBS' for the given `<PC-NAME>.pc' file.

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

# Sets `NIX_CFLAGS' and `NIX_LIBS'
FLOX_CHECK_NIX_MODULES


# ---------------------------------------------------------------------------- #

# Nix CPP Flags
# -------------

# Compiler Flags for pre-processing with `nix' headers

AC_MSG_CHECKING([Nix system pair])
AC_ARG_VAR([SYSTEM], [Nix system pair])
SYSTEM="$OS-$ARCH";
AC_MSG_RESULT([$SYSTEM])
AC_SUBST([SYSTEM])

AC_DEFINE_UNQUOTED([SYSTEM], ["$SYSTEM"], [Nix system pair])


# Check configuration used to compile `nix' and add it to `DEFS' ( `CPPFLAGS' ).
FLOX_INHERIT_NIX_CONFIG_DEF([HAVE_BOEHMGC], [1])
FLOX_INHERIT_NIX_CONFIG_DEF([ENABLE_S3], [1])
FLOX_INHERIT_NIX_CONFIG_DEF([HAVE_STRUCT_DIRENT_D_TYPE], [1])
FLOX_INHERIT_NIX_CONFIG_DEF([EXCEPTION_NEEDS_THROW_SPEC], [1])
FLOX_INHERIT_NIX_CONFIG_DEF([HAVE_BOOST])

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

# `RPATH', `RUNPATH', and `SONAME' Flags
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

# Library Flags
# -------------
# - `SHARED_LIBRARY_LDFLAGS' is used to link shared libraries.
# - `NO_UNDEFINED_SYMBOLS_FLAG' disallows undefined symbols in shared libraries.

AS_CASE(["$CXX_TOOLCHAIN"],
  [gcc],   [NO_UNDEFINED_SYMBOLS_FLAG='-Wl,--no-undefined'],
  [clang], [NO_UNDEFINED_SYMBOLS_FLAG='-Wl,-undefined,error']
  [AC_MSG_ERROR([Unsupported C++ compiler: $CXX])])
AC_SUBST([NO_UNDEFINED_SYMBOLS_FLAG])

AM_CONDITIONAL([BUILD_SHARED_LIBS], [test "$enable_shared" = 'yes'])


# ---------------------------------------------------------------------------- #

# Sub-System Versions
# -------------------

# Get `pkgdb' version

AC_SUBST([PKGDB_VERSION], ["$(< $PKGDB_ROOT/.version)"])
AC_DEFINE_UNQUOTED([PKGDB_VERSION], ["$PKGDB_VERSION"], [Version of pkgdb])


# ---------------------------------------------------------------------------- #

# Extra Files/Assets
# ------------------

# Get paths to static assets

AC_ARG_VAR([SET_PROMPT_BASH_SH],
  [Path to 'set-prompt.bash.sh' script in '/nix/store/'])

# --with-set-prompt-bash-sh
AC_ARG_WITH([set-prompt-bash-sh],
  [AS_HELP_STRING(
    [--with-set-prompt-bash-sh=local|PATH],
    [use set-prompt-bash.sh from local tree, or a '/nix/store' path ]
    [@<:@default=local@:>@])],
  [WITH_SET_PROMPT_BASH_SH="$withval"],
  [WITH_SET_PROMPT_BASH_SH=local])
AS_IF([test -z "${SET_PROMPT_BASH_SH:-}"], [
  AS_CASE(["$WITH_SET_PROMPT_BASH_SH"],
    [/nix/store/*], [SET_PROMPT_BASH_SH="$WITH_SET_PROMPT_BASH_SH"],
    [local],
      [SET_PROMPT_BASH_SH="$(
       $NIX $NIXFLAGS store add-path -n set-prompt-bash.sh  \
            "$PKGDB_ROOT/src/buildenv/set-prompt-bash.sh";
       )"],
    [AS_IF([test -d "$WITH_SET_PROMPT_BASH_SH"], [:],
      [AC_MSG_ERROR(
        [Invalid '--with-set-prompt-bash-sh': '$WITH_SET_PROMPT_BASH_SH'. ]
        [You must specify either 'local', or a '/nix/store' path])])])])
AC_SUBST([WITH_SET_PROMPT_BASH_SH])
AC_DEFINE_UNQUOTED([SET_PROMPT_BASH_SH], ["$SET_PROMPT_BASH_SH"],
  [Path to 'set-prompt.bash.sh' script in '/nix/store/'])


AC_ARG_VAR([PROFILE_D_SCRIPT_DIR],
  [Path to directory containing 'etc/profile.d' directory in '/nix/store/'])

# --with-profile-d-script-dir
AC_ARG_WITH([profile-d-script-dir],
  [AS_HELP_STRING(
    [--with-profile-d-script-dir=local|PATH],
    [use 'etc/profile.d' from local tree, or a '/nix/store' path ]
    [@<:@default=local@:>@])],
  [WITH_PROFILE_D_SCRIPT_DIR="$withval"],
  [WITH_PROFILE_D_SCRIPT_DIR=local])
AS_IF([test -z "${PROFILE_D_SCRIPT_DIR:-}"], [
  AS_CASE(["$WITH_PROFILE_D_SCRIPT_DIR"],
    [/nix/store/*], [PROFILE_D_SCRIPT_DIR="$WITH_PROFILE_D_SCRIPT_DIR"],
    [local],
      [PROFILE_D_SCRIPT_DIR="$(
        $NIX $NIXFLAGS store add-file -n set-prompt-bash.sh  \
             "$PKGDB_ROOT/src/buildenv/assets";
        )"],
    [AS_IF([test -d "$WITH_PROFILE_D_SCRIPT_DIR"], [:],
      [AC_MSG_ERROR(
        [Invalid '--with-profile-d-script-dir': '$WITH_PROFILE_D_SCRIPT_DIR'. ]
        [You must specify either 'local', or a '/nix/store' path])])])])
AC_SUBST([WITH_PROFILE_D_SCRIPT_DIR])
AC_DEFINE_UNQUOTED([PROFILE_D_SCRIPT_DIR], ["$PROFILE_D_SCRIPT_DIR"],
  [Path to directory containing 'etc/profile.d' directory in '/nix/store/'])


# ---------------------------------------------------------------------------- #

# Node Semver
# -----------

# Get path to `semver' in `/nix/store'

AC_ARG_VAR([SEMVER_PATH], [Path to 'semver' executable in '/nix/store/'])
# NOTE: `@%:@' is replaced with `#' by `autoconf' so that it is not treated as
#       a comment.
AS_CASE(["$SEMVER"],
  [/nix/store/*], [SEMVER_PATH="$SEMVER"],
  [SEMVER_PATH="$(
     $NIX $NIXFLAGS build --no-link --print-out-paths  \
          'github:aakropotkin/floco@%:@semver'
   )/bin/semver"])
AC_SUBST([SEMVER_PATH])
AC_DEFINE_UNQUOTED([SEMVER_PATH], ["$SEMVER_PATH"],
  [Path to 'semver' executable in '/nix/store'])


# ---------------------------------------------------------------------------- #

# Finalize C/C++ Flags
# --------------------
# We use `AM_CFLAGS' and `AM_CXXFLAGS' for things we don't want users to
# override, which allows users to add their own flags to `CXXFLAGS'
# and `LDFLAGS'.

# User Overridable flags.
# We provide a sane default, but users are encouraged to override these.
CXXFLAGS="${CXXFLAGS:+$CXXFLAGS }-Wall -Wextra -Werror -pedantic";
AS_IF([test "$CXX_TOOLCHAIN" = 'clang'],
      [CXXFLAGS="$CXXFLAGS -Wno-gnu-zero-variadic-macro-arguments"])

#NOTE: `YAML_*' and `TOML_*' are only used by some targets, so are excluded here

# Set `CXXFLAGS' used for all sub-systems.
AM_CXXFLAGS="${AM_CXXFLAGS:+$AM_CXXFLAGS }-I$REPO_ROOT/pkgdb/include";
AM_CXXFLAGS="$AM_CXXFLAGS $NIX_CFLAGS $NLJSON_CFLAGS $ARGPARSE_CFLAGS";
AM_CXXFLAGS="$AM_CXXFLAGS $SQLITE3PP_CFLAGS $SQLITE3_CFLAGS $YAML_CFLAGS";
AC_SUBST([AM_CXXFLAGS])

# Set `LDFLAGS' used for all sub-systems.
AM_LDFLAGS="${AM_LDFLAGS:+$AM_LDFLAGS }$NIX_LIBS $NLJSON_LIBS $ARGPARSE_LIBS";
AM_LDFLAGS="$AM_LDFLAGS $SQLITE3PP_LIBS $SQLITE3_LIBS";
AC_SUBST([AM_LDFLAGS])


# ---------------------------------------------------------------------------- #


# Create Symlinks into Build Area
# -------------------------------

AC_CONFIG_LINKS([
  pkgdb/tests/setup_suite.bash:pkgdb/tests/setup_suite.bash
])


# ---------------------------------------------------------------------------- #

# Perform Substitution
# --------------------
# Creates `FILE' from `FILE.in' by substituting `@VAR@' for `$VAR'.
# Only _substituted_ variables ( those marked by `AC_SUBST' ) are subtituted.

# Locate files to be processed ( `.in' suffix is implied ).
AC_CONFIG_FILES([
  Makefile
  pkgdb/Makefile
  pkgdb/src/resolver/Makefile
  pkgdb/include/Makefile
  pkgdb/tests/Makefile
  cli/Makefile
], [
$MKDIR_P pkgdb/tests;
$LN_S -f "$PKGDB_ROOT/tests/data"      pkgdb/tests/data;
$LN_S -f "$PKGDB_ROOT/tests/harnesses" pkgdb/tests/harnesses;
])

# Substitute and write.
AC_OUTPUT


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
