# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../../build-aux/m4

# ---------------------------------------------------------------------------- #

$(abs_top_builddir)/pkgdb/lib/libpkgdb.la:
	$(MAKE) -C $(abs_top_builddir)/pkgdb lib/libpkgdb.la

$(abs_top_builddir)/pkgdb/bin/pkgdb$(EXEEXT):
	$(MAKE) -C $(abs_top_builddir)/pkgdb bin/pkgdb$(EXEEXT)

# Causes `check-am' to depend on `pkgdb' being built.
# There isn't another clear way to declare this dependency without causing
# issues with automatic dependency tracking.
all-local: $(abs_top_builddir)/pkgdb/bin/pkgdb$(EXEEXT)


# ---------------------------------------------------------------------------- #

TEST_DATA_DIR           = $(PKGDB_TEST_DIR)/data
AM_CPPFLAGS             = -DTEST_DATA_DIR=\"$(TEST_DATA_DIR)\"
PKGDB_BIN               = $(abs_top_builddir)/pkgdb/bin/pkgdb
PKGDB_IS_SQLITE3_BIN    = $(abs_builddir)/is_sqlite3
PKGDB_SEARCH_PARAMS_BIN = $(abs_builddir)/search-params

AM_TESTS_ENVIRONMENT =
AM_TESTS_ENVIRONMENT += export TEST_DATA_DIR="$(TEST_DATA_DIR)";
AM_TESTS_ENVIRONMENT += export PKGDB_BIN="$(PKGDB_BIN)";
AM_TESTS_ENVIRONMENT += export PKGDB_IS_SQLITE3_BIN="$(PKGDB_IS_SQLITE3_BIN)";
AM_TESTS_ENVIRONMENT += PKGDB_SEARCH_PARAMS_BIN="$(PKGDB_SEARCH_PARAMS_BIN)";
AM_TESTS_ENVIRONMENT += export PKGDB_SEARCH_PARAMS_BIN;


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


.PHONY: tests
tests: $(check_PROGRAMS)


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

environment_LDADD = ../lib/libpkgdb.la
exceptions_LDADD  = ../lib/libpkgdb.la
gc_LDADD          = ../lib/libpkgdb.la
lockfile_LDADD    = ../lib/libpkgdb.la
manifest_LDADD    = ../lib/libpkgdb.la
pkgdb_LDADD       = ../lib/libpkgdb.la
registry_LDADD    = ../lib/libpkgdb.la
util_LDADD        = ../lib/libpkgdb.la
versions_LDADD    = ../lib/libpkgdb.la

is_sqlite3_LDADD    = ../lib/libpkgdb.la
search_params_LDADD = ../lib/libpkgdb.la


# ---------------------------------------------------------------------------- #

TESTS =
TESTS += environment
TESTS += exceptions
TESTS += gc
TESTS += lockfile
TESTS += manifest
TESTS += pkgdb
TESTS += registry
TESTS += util
TESTS += versions

TESTS += descriptor.bats
TESTS += migrate.bats
TESTS += scrape.bats
TESTS += get.bats
TESTS += cli.bats
TESTS += ga-registry.bats
TESTS += search.bats
TESTS += pathological.bats
TESTS += groups.bats
TESTS += buildenv.bats
TESTS += gc.bats
TESTS += sqlite3.bats


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
