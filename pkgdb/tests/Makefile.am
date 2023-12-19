# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../../build-aux/m4

# ---------------------------------------------------------------------------- #

check_PROGRAMS =
check_PROGRAMS += environment
check_PROGRAMS += exceptions
check_PROGRAMS += gc
check_PROGRAMS += lockfile
check_PROGRAMS += manifest
check_PROGRAMS += pkgdb
check_PROGRAMS += registry
check_PROGRAMS += util
check_PROGRAMS += versions

check_PROGRAMS += is_sqlite3
check_PROGRAMS += search-params


# ---------------------------------------------------------------------------- #

environment_SOURCES = environment.cc test.hh
exceptions_SOURCES  = exceptions.cc test.hh
gc_SOURCES          = gc.cc test.hh
lockfile_SOURCES    = lockfile.cc test.hh
manifest_SOURCES    = manifest.cc test.hh
pkgdb_SOURCES       = pkgdb.cc test.hh
registry_SOURCES    = registry.cc test.hh
util_SOURCES        = util.cc test.hh
versions_SOURCES    = versions.cc test.hh

is_sqlite3_SOURCES    = is_sqlite3.cc
search_params_SOURCES = search-params.cc


# ---------------------------------------------------------------------------- #

AM_CPPFLAGS = $(AM_CXXFLAGS)
AM_LDFLAGS  = $(AM_LDFLAGS)

environment_LDADD = ../libpkgdb.la
exceptions_LDADD  = ../libpkgdb.la
gc_LDADD          = ../libpkgdb.la
lockfile_LDADD    = ../libpkgdb.la
manifest_LDADD    = ../libpkgdb.la
pkgdb_LDADD       = ../libpkgdb.la
registry_LDADD    = ../libpkgdb.la
util_LDADD        = ../libpkgdb.la
versions_LDADD    = ../libpkgdb.la

is_sqlite3_LDADD    = ../libpkgdb.la
search_params_LDADD = ../libpkgdb.la


# ---------------------------------------------------------------------------- #

# TODO: bats

TESTS =
TESTS += environment
TESTS += exceptions
TESTS += gc
TESTS += lockfile
TESTS += manifest
TESTS += pkgdbT
TESTS += registry
TESTS += util
TESTS += versions


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
