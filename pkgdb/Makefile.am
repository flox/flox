# ============================================================================ #
#
# @file pkgdb/src/resolver/Makefile.am
#
# @brief Provides `libpkgdb.la` module and `pkgdb` executable.
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../build-aux/m4

SUBDIRS = src/resolver include tests

# ---------------------------------------------------------------------------- #

src/resolver/libresolver.la:
	$(MAKE) -C src/resolver libresolver.la


# ---------------------------------------------------------------------------- #

lib_LTLIBRARIES = lib/libpkgdb.la

lib_libpkgdb_la_CXXFLAGS =  $(AM_CXXFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(YAML_CFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(TOML_CFLAGS)

lib_libpkgdb_la_LDFLAGS =  $(AM_LDFLAGS)
lib_libpkgdb_la_LDFLAGS += '$(set_soname_option)libpkgdb$(sharedLibExt)'
if IS_LINUX
lib_libpkgdb_la_LDFLAGS += $(RELATIVE_RPATH_FLAG)
endif IS_LINUX
lib_libpkgdb_la_LDFLAGS += $(NO_UNDEFINED_SYMBOLS_FLAG)
lib_libpkgdb_la_LDFLAGS += $(YAML_LIBS)
lib_libpkgdb_la_LDFLAGS += $(TOML_LIBS)

lib_libpkgdb_la_LIBADD = src/resolver/libresolver.la

lib_libpkgdb_la_SOURCES =
lib_libpkgdb_la_SOURCES += src/command.cc
lib_libpkgdb_la_SOURCES += src/eval.cc
lib_libpkgdb_la_SOURCES += src/exceptions.cc
lib_libpkgdb_la_SOURCES += src/expr.cc
lib_libpkgdb_la_SOURCES += src/flake-package.cc
lib_libpkgdb_la_SOURCES += src/flox-flake.cc
lib_libpkgdb_la_SOURCES += src/logger.cc
lib_libpkgdb_la_SOURCES += src/nix-state.cc
lib_libpkgdb_la_SOURCES += src/package.cc
lib_libpkgdb_la_SOURCES += src/raw-package.cc
lib_libpkgdb_la_SOURCES += src/registry.cc
lib_libpkgdb_la_SOURCES += src/tomlToJSON.cc
lib_libpkgdb_la_SOURCES += src/util.cc
lib_libpkgdb_la_SOURCES += src/versions.cc
lib_libpkgdb_la_SOURCES += src/yamlToJSON.cc

lib_libpkgdb_la_SOURCES += src/search/params.cc

lib_libpkgdb_la_SOURCES += src/pkgdb/db-package.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/gc.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/input.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/list.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/pkg-query.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/primops.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/read.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/write.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/schemas.hh

lib_libpkgdb_la_SOURCES += src/buildenv/buildenv.cc
lib_libpkgdb_la_SOURCES += src/buildenv/realise.cc


# ---------------------------------------------------------------------------- #

bin_PROGRAMS = bin/pkgdb

bin_pkgdb_LDADD =  $(AM_LDFLAGS)
bin_pkgdb_LDADD += $(RELATIVE_RPATH_FLAG)
bin_pkgdb_LDADD += lib/libpkgdb.la

bin_pkgdb_SOURCES =
bin_pkgdb_SOURCES += src/main.cc
bin_pkgdb_SOURCES += src/repl.cc
bin_pkgdb_SOURCES += src/parse/command.cc
bin_pkgdb_SOURCES += src/pkgdb/command.cc
bin_pkgdb_SOURCES += src/pkgdb/get.cc
bin_pkgdb_SOURCES += src/pkgdb/scrape.cc
bin_pkgdb_SOURCES += src/resolver/command.cc
bin_pkgdb_SOURCES += src/search/command.cc
bin_pkgdb_SOURCES += src/buildenv/command.cc


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
