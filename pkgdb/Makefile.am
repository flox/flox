# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../build-aux/m4

# TODO:
#SUBDIRS = include tests


# ---------------------------------------------------------------------------- #

# TODO: Set this in `configure.ac'.
#       It improves visiblity of the defaults from `--help'.

if IS_CLANG
EXTRA_CXXFLAGS ?= -Wall -Wextra -Werror -pedantic         \
                  -Wno-gnu-zero-variadic-macro-arguments
else  # GCC
EXTRA_CXXFLAGS ?= -Wall -Wextra -Werror -pedantic
endif # if IS_CLANG

EXTRA_LDFLAGS ?=


# ---------------------------------------------------------------------------- #

AM_CXXFLAGS =
AM_CXXFLAGS += $(EXTRA_CXXFLAGS)
AM_CXXFLAGS += -I$(PKGDB_ROOT)/include

AM_LDFLAGS =
AM_LDFLAGS += $(EXTRA_LDFLAGS)


# ---------------------------------------------------------------------------- #

lib_LTLIBRARIES = lib/libpkgdb.la

bin_PROGRAMS =
bin_PROGRAMS += bin/pkgdb


# ---------------------------------------------------------------------------- #

lib_libpkgdb_la_CXXFLAGS =
lib_libpkgdb_la_CXXFLAGS += $(AM_CXXFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(NLJSON_CFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(NIX_CFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(YAML_CFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(TOML_CFLAGS)
lib_libpkgdb_la_CXXFLAGS += $(SQLITE3PP_CFLAGS)
# FIXME: This should only apply to shared libraries.
lib_libpkgdb_la_CXXFLAGS += $(SHARED_LIBRARY_CXXFLAGS)

lib_libpkgdb_la_LDFLAGS =
lib_libpkgdb_la_LDFLAGS += $(AM_LDFLAGS)
lib_libpkgdb_la_LDFLAGS += '$(set_soname_option)libpkgdb$(sharedLibExt)'
lib_libpkgdb_la_LDFLAGS += $(NLJSON_LIBS)
lib_libpkgdb_la_LDFLAGS += $(NIX_LIBS)
lib_libpkgdb_la_LDFLAGS += $(YAML_LIBS)
lib_libpkgdb_la_LDFLAGS += $(TOML_LIBS)
lib_libpkgdb_la_LDFLAGS += $(SQLITE3PP_LIBS)

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

lib_libpkgdb_la_SOURCES += src/resolver/descriptor.cc
lib_libpkgdb_la_SOURCES += src/resolver/environment.cc
lib_libpkgdb_la_SOURCES += src/resolver/lockfile.cc
lib_libpkgdb_la_SOURCES += src/resolver/manifest-raw.cc
lib_libpkgdb_la_SOURCES += src/resolver/manifest.cc
lib_libpkgdb_la_SOURCES += src/resolver/mixins.cc
lib_libpkgdb_la_SOURCES += src/resolver/primops.cc

lib_libpkgdb_la_SOURCES += src/pkgdb/db-package.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/gc.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/input.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/list.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/pkg-query.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/primops.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/read.cc
lib_libpkgdb_la_SOURCES += src/pkgdb/write.cc

lib_libpkgdb_la_SOURCES += src/buildenv/buildenv.cc
lib_libpkgdb_la_SOURCES += src/buildenv/realise.cc


# ---------------------------------------------------------------------------- #

# FIXME: `RPATH'
bin_pkgdb_CXXFLAGS =
bin_pkgdb_CXXFLAGS += $(AM_CXXFLAGS)
bin_pkgdb_CXXFLAGS += $(NLJSON_CFLAGS)
bin_pkgdb_CXXFLAGS += $(NIX_CFLAGS)
bin_pkgdb_CXXFLAGS += $(ARGPARSE_CFLAGS)

bin_pkgdb_LDADD =
bin_pkgdb_LDADD += $(AM_LDFLAGS)
bin_pkgdb_LDADD += $(RELATIVE_RPATH_FLAG)
bin_pkgdb_LDADD += lib/libpkgdb.la
bin_pkgdb_LDADD += $(NLJSON_LIBS)
bin_pkgdb_LDADD += $(NIX_LIBS)
bin_pkgdb_LDADD += $(ARGPARSE_LIBS)
# For `libpkgdb.a'
bin_pkgdb_LDADD += $(YAML_LIBS)
bin_pkgdb_LDADD += $(TOML_LIBS)
bin_pkgdb_LDADD += $(SQLITE3PP_LIBS)

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
