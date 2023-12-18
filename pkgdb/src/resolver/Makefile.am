# ============================================================================ #
#
# @file pkgdb/src/resolver/Makefile.am
#
# @brief Provides `libresolver.la' module.
#
#
# ---------------------------------------------------------------------------- #

ACLOCAL_AMFLAGS = -I ../../../build-aux/m4


# ---------------------------------------------------------------------------- #

noinst_LTLIBRARIES = libresolver.la

libresolver_la_CXXFLAGS =
libresolver_la_CXXFLAGS += $(AM_CXXFLAGS)
libresolver_la_CXXFLAGS += -I$(PKGDB_ROOT)/include
libresolver_la_CXXFLAGS += $(SQLITE3PP_CFLAGS)
libresolver_la_CXXFLAGS += $(NIX_CFLAGS)

libresolver_la_LDFLAGS =
libresolver_la_LDFLAGS += $(AM_LDFLAGS)
libresolver_la_LDFLAGS += $(SQLITE3PP_LIBS)
libresolver_la_LDFLAGS += $(NIX_LIBS)

libresolver_la_SOURCES =
libresolver_la_SOURCES += descriptor.cc
libresolver_la_SOURCES += environment.cc
libresolver_la_SOURCES += lockfile.cc
libresolver_la_SOURCES += manifest-raw.cc
libresolver_la_SOURCES += manifest.cc
libresolver_la_SOURCES += mixins.cc
libresolver_la_SOURCES += primops.cc



# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
