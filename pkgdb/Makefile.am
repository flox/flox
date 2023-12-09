# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../build-aux/m4

SUBDIRS = include tests


# ---------------------------------------------------------------------------- #

AM_CXXFLAGS =
AM_CXXFLAGS += -Iinclude


# ---------------------------------------------------------------------------- #

# Used to guess the source filename for an executable.
# This is only used for `tests/*.cc' files.
AM_DEFAULT_SOURCE_EXT = .cc

noinst_LIBRARIES =
noinst_LIBRARIES += lib/libpkgdb.a

lib_sharedLIBRARIES = lib/libpkgdb$(sharedLibExt)

bin_PROGRAMS =
bin_PROGRAMS += bin/pkgdb


# ---------------------------------------------------------------------------- #

lib_libpkgdb_a_CXXFLAGS =
lib_libpkgdb_a_CXXFLAGS += $(AM_CXXFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(SHARED_LIBRARY_CXXFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(NLJSON_CFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(NIX_CFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(YAML_CFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(TOML_CFLAGS)
lib_libpkgdb_a_CXXFLAGS += $(SQLITE3PP_CFLAGS)

lib_libpkgdb_a_SOURCES =
lib_libpkgdb_a_SOURCES += src/command.cc
lib_libpkgdb_a_SOURCES += src/eval.cc
lib_libpkgdb_a_SOURCES += src/exceptions.cc
lib_libpkgdb_a_SOURCES += src/expr.cc
lib_libpkgdb_a_SOURCES += src/flake-package.cc
lib_libpkgdb_a_SOURCES += src/flox-flake.cc
lib_libpkgdb_a_SOURCES += src/logger.cc
lib_libpkgdb_a_SOURCES += src/nix-state.cc
lib_libpkgdb_a_SOURCES += src/package.cc
lib_libpkgdb_a_SOURCES += src/raw-package.cc
lib_libpkgdb_a_SOURCES += src/registry.cc
lib_libpkgdb_a_SOURCES += src/tomlToJSON.cc
lib_libpkgdb_a_SOURCES += src/util.cc
lib_libpkgdb_a_SOURCES += src/versions.cc
lib_libpkgdb_a_SOURCES += src/yamlToJSON.cc

lib_libpkgdb_a_SOURCES += src/search/params.cc

lib_libpkgdb_a_SOURCES += src/resolver/descriptor.cc
lib_libpkgdb_a_SOURCES += src/resolver/environment.cc
lib_libpkgdb_a_SOURCES += src/resolver/lockfile.cc
lib_libpkgdb_a_SOURCES += src/resolver/manifest-raw.cc
lib_libpkgdb_a_SOURCES += src/resolver/manifest.cc
lib_libpkgdb_a_SOURCES += src/resolver/mixins.cc
lib_libpkgdb_a_SOURCES += src/resolver/primops.cc

lib_libpkgdb_a_SOURCES += src/pkgdb/db-package.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/gc.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/input.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/list.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/pkg-query.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/primops.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/read.cc
lib_libpkgdb_a_SOURCES += src/pkgdb/write.cc


# ---------------------------------------------------------------------------- #

bin_pkgdb_CXXFLAGS =
bin_pkgdb_CXXFLAGS += $(AM_CXXFLAGS)
bin_pkgdb_CXXFLAGS += $(NLJSON_CFLAGS)
bin_pkgdb_CXXFLAGS += $(NIX_CFLAGS)
bin_pkgdb_CXXFLAGS += $(ARGPARSE_CFLAGS)

bin_pkgdb_LDADD =
bin_pkgdb_LDADD += lib/libpkgdb.a
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


# ---------------------------------------------------------------------------- #

# NOTE: There is no `libpkgs_sa_CXXFLAGS', use `libpkgdb_a_CXXFLAGS' instead.

libpkgdb_sa_LIBS =
libpkgdb_sa_LIBS += $(AM_CXXLIBS)
libpkgdb_sa_LIBS += $(SHARED_LIBRARY_LDFLAGS)
libpkgdb_sa_LIBS += '$(set_soname_option)libpkgdb$(sharedLibExt)'
libpkgdb_sa_LIBS += $(NLJSON_LIBS)
libpkgdb_sa_LIBS += $(NIX_LIBS)
libpkgdb_sa_LIBS += $(YAML_LIBS)
libpkgdb_sa_LIBS += $(TOML_LIBS)
libpkgdb_sa_LIBS += $(SQLITE3PP_LIBS)

lib/libpkgdb$(sharedLibExt): lib/libpkgdb.a
	$(CXXLINK) $(libpkgdb_sa_LIBS) -o $@ $^


# ---------------------------------------------------------------------------- #

# Extend normal installation to add shared library
install-exec-local-shared-lib: lib/libpkgdb$(sharedLibExt)
	@$(NORMAL_INSTALL)
	$(MKDIR_P) $(DESTDIR)$(libdir)
	$(INSTALL_PROGRAM_ENV)                                      \
	  $(INSTALL_PROGRAM) lib/libpkgdb$(sharedLibExt)            \
                         "$(DESTDIR)$(libdir)"||exit $$?;

# TODO: Install `lib/pkgconfig/libpkgdb.pc' file.

install-exec-local: install-exec-local-shared-lib


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
