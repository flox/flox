# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../m4

SUBDIRS = tests


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

nobase_include_HEADERS =

nobase_include_HEADERS += include/versions.hh

nobase_include_HEADERS += include/compat/concepts.hh

nobase_include_HEADERS += include/flox/flake-package.hh
nobase_include_HEADERS += include/flox/raw-package.hh
nobase_include_HEADERS += include/flox/package.hh
nobase_include_HEADERS += include/flox/flox-flake.hh
nobase_include_HEADERS += include/flox/registry.hh
nobase_include_HEADERS += include/flox/eval.hh
nobase_include_HEADERS += include/flox/repl.hh

nobase_include_HEADERS += include/flox/core/expr.hh
nobase_include_HEADERS += include/flox/core/nix-state.hh
nobase_include_HEADERS += include/flox/core/types.hh
nobase_include_HEADERS += include/flox/core/exceptions.hh
nobase_include_HEADERS += include/flox/core/util.hh
nobase_include_HEADERS += include/flox/core/command.hh

nobase_include_HEADERS += include/flox/search/params.hh
nobase_include_HEADERS += include/flox/search/command.hh

nobase_include_HEADERS += include/flox/parse/command.hh

nobase_include_HEADERS += include/flox/resolver/environment.hh
nobase_include_HEADERS += include/flox/resolver/mixins.hh
nobase_include_HEADERS += include/flox/resolver/lockfile.hh
nobase_include_HEADERS += include/flox/resolver/manifest-raw.hh
nobase_include_HEADERS += include/flox/resolver/manifest.hh
nobase_include_HEADERS += include/flox/resolver/descriptor.hh
nobase_include_HEADERS += include/flox/resolver/command.hh
nobase_include_HEADERS += include/flox/resolver/primops.hh

nobase_include_HEADERS += include/flox/pkgdb/read.hh
nobase_include_HEADERS += include/flox/pkgdb/gc.hh
nobase_include_HEADERS += include/flox/pkgdb/input.hh
nobase_include_HEADERS += include/flox/pkgdb/pkg-query.hh
nobase_include_HEADERS += include/flox/pkgdb/db-package.hh
nobase_include_HEADERS += include/flox/pkgdb/command.hh
nobase_include_HEADERS += include/flox/pkgdb/primops.hh
nobase_include_HEADERS += include/flox/pkgdb/write.hh


# ---------------------------------------------------------------------------- #

lib_libpkgdb_a_CXXFLAGS =
lib_libpkgdb_a_CXXFLAGS += $(AM_CXXFLAGS)
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

if IS_GCC
no_undefined_LDFLAG = -Wl,--no-undefined
else
no_undefined_LDFLAG = -Wl,-undefined,error
endif

libpkgdb_sa_LIBS =
if IS_LINUX
relative_rpath_LDFLAG  =  -Wl,--enable-new-dtags,--rpath,'$$ORIGIN/../lib'
libpkgdb_sa_LIBS += '-Wl,-soname,libpkgdb$(sharedLibExt)'
else
relative_rpath_LDFLAG  =  -rpath @executable_path/../lib
libpkgdb_sa_LIBS += -install_name '@rpath/libpkgdb$(sharedLibExt)'
endif
libpkgdb_sa_LIBS += $(relative_rpath_LDFLAG)

libpkgdb_sa_LIBS += $(NLJSON_LIBS)
libpkgdb_sa_LIBS += $(NIX_LIBS)
libpkgdb_sa_LIBS += $(YAML_LIBS)
libpkgdb_sa_LIBS += $(TOML_LIBS)
libpkgdb_sa_LIBS += $(SQLITE3PP_LIBS)

lib/libpkgdb$(sharedLibExt): CXXFLAGS += -fPIC
lib/libpkgdb$(sharedLibExt): LDFLAGS  += -fPIC -shared $(no_undefined_LDFLAG)
lib/libpkgdb$(sharedLibExt): lib/libpkgdb.a
	$(CXXLINK) $(libpkgdb_sa_LIBS) -o $@ $^


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
