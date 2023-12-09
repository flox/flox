# ============================================================================ #
#
# @file pkgdb/Include.mk
#
# @brief Provides a set of Makefile variables for use in `env-builder'.
#
# These populate settings needed by this project's `Makefile' and the project
# root `Makefile'.
#
#
# ---------------------------------------------------------------------------- #

ifndef __PKGDB_MK
__PKGDB_MK = 1

# ---------------------------------------------------------------------------- #

MAKEFILE_DIR  ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
REPO_ROOT     ?= $(patsubst %/,%,$(dir $(MAKEFILE_DIR)))
REPO_ROOT     := $(REPO_ROOT)
BUILD_AUX_DIR ?= $(REPO_ROOT)/build-aux
MK_DIR        ?= $(BUILD_AUX_DIR)/mk

ifeq (,$(wildcard $(REPO_ROOT)/pkgdb/))
$(error "Unable to locate repository root")
endif


# ---------------------------------------------------------------------------- #

include $(MK_DIR)/platform.mk
include $(MK_DIR)/lib.mk
include $(MK_DIR)/files.mk
include $(MK_DIR)/flags.mk

# ---------------------------------------------------------------------------- #

# Initialize project variables.
$(eval $(call def_cxx_project,env_builder,pkgdb))


# ---------------------------------------------------------------------------- #

pkgdb_LIBS = lib/libpkgdb$(libExt)
pkgdb_BINS = bin/pkgdb

pkgdb_HEADERS = $(call rwildcard,$(PKGDB_ROOT)include,*.hh)
pkgdb_SRCS    = $(call rwildcard,$(PKGDB_ROOT)include,*.cc)

pkgdb_TEST_BINS = $(patsubst %.cc,%,$(wildcard $(PKGDB_ROOT)tests/*.cc))
pkgdb_TESTS	    = $(filter-out is_sqlite3 search-params,$(pkgdb_TEST_BINS))


# ---------------------------------------------------------------------------- #

pkgdb_CLEANFILES += lib/pkgconfig/flox-pkgdb.pc
pkgdb_CLEANFILES += $(addprefix $(PKGDB_ROOT)/docs/,*.png *.html *.svg *.css)
pkgdb_CLEANFILES += $(addprefix $(PKGDB_ROOT)/docs/,*.js)
pkgdb_CLEANDIRS  += docs/search


# ---------------------------------------------------------------------------- #

# Check External Dependency flags
# -------------------------------

ifeq (,$(nlohmann_json_CFLAGS))
$(error You must set 'nlohmann_json_CFLAGS')
endif  # ifeq (,$(nlohmann_json_CFLAGS))


ifeq (,$(argparse_CFLAGS))
$(error You must set 'argparse_CFLAGS')
endif  # ifeq (,$(argparse_CFLAGS))


ifeq (,$(boost_CFLAGS))
$(error You must set 'boost_CFLAGS')
endif  # ifeq (,$(boost_CFLAGS))


ifeq (,$(toml_CFLAGS))
$(error You must set 'toml_CFLAGS')
endif  # ifeq (,$(toml_CFLAGS))


ifeq (,$(sqlite3_CFLAGS))
$(error You must set 'sqlite3_CFLAGS')
endif  # ifeq (,$(sqlite3_CFLAGS))

ifeq (,$(sqlite3_LIBS))
$(error You must set 'sqlite3_LIBS')
endif  # ifeq (,$(sqlite3_LIBS))

ifeq (,$(sqlite3pp_CFLAGS))
$(error You must set 'sqlite3pp_CFLAGS')
endif  # ifeq (,$(sqlite3pp_CFLAGS))


ifeq (,$(yaml_CFLAGS))
$(error You must set 'yaml_CFLAGS')
endif  # ifeq (,$(yaml_CFLAGS))

ifeq (,$(yaml_LIBS))
$(error You must set 'yaml_LIBS')
endif  # ifeq (,$(yaml_LIBS))


ifeq (,$(nix_INCDIR))
$(error Unable to locate 'nix' include directory)
endif  # ifeq (,$(nix_INCDIR))

ifeq (,$(nix_LIBDIR))
$(error You must set 'nix_LIBDIR')
endif  # ifeq (,$(nix_LIBDIR))

ifeq (,$(nix_CFLAGS))
$(error You must set 'nix_CFLAGS')
endif  # ifeq (,$(nix_CFLAGS))

ifeq (,$(nix_LIBS))
$(error You must set 'nix_LIBS')
endif  # ifeq (,$(nix_LIBS))


ifeq (,$(libpkgdb_CXXFLAGS))
libpkgdb_CXXFLAGS =
libpkgdb_CXXFLAGS += -I$(if $(wildcard),$(INCLUDEDIR),$(INCLUDEDIR),\
                                                      $(PKGDB_ROOT)include)
libpkgdb_CXXFLAGS += $(nix_CFLAGS)
libpkgdb_CXXFLAGS += -include $(nix_INCDIR)/nix/config.h
libpkgdb_CXXFLAGS += $(nlohmann_json_CFLAGS)
libpkgdb_CXXFLAGS += $(sqlite3pp_CFLAGS)
libpkgdb_CXXFLAGS += $(argparse_CFLAGS)
endif  # ifeq (,$(libpkgdb_CXXFLAGS))

ifeq (,$(libpkgdb_LIBS))
libpkgdb_LIBS =
libpkgdb_LIBS += -L$(if $(wildcard $(LIBDIR),$(LIBDIR),$(PKGDB_ROOT)/lib))
libpkgdb_LIBS += $(call set_rpath,$(if $(wildcard $(LIBDIR),$(LIBDIR),\
                                                            $(PKGDB_ROOT)/lib)))
libpkgdb_LIBS += -lpkgdb
libpkgdb_LIBS += $(nix_LIBS) -lnixfetchers
libpkgdb_LIBS += $(sqlite3pp_LIBS) $(sqlite3_LIBS) $(argparse_LIBS)
endif  # ifeq (,$(libpkgdb_LIBS))


# ---------------------------------------------------------------------------- #

# Locate `semver'
# ---------------

# TODO: Don't use `nix build'
SEMVER_PATH ?=                                                        \
  $(shell $(NIX) build --no-link --print-out-paths                    \
	                     'github:aakropotkin/floco#semver')/bin/semver
pkgdb_CXXFLAGS += '-DSEMVER_PATH="$(SEMVER_PATH)"'


pkgdb_CXXFLAGS += $(libpkgdb_CXXFLAGS) $(toml_CFLAGS) $(yaml_CFLAGS)
pkgdb_LDFLAGS  += $(libpkgdb_LIBS) $(yaml_LIBS)


# ---------------------------------------------------------------------------- #

ifeq (,$(pkgdb_PC_CFLAGS))
pkgdb_PC_CFLAGS =  $(lastword $(filter -std=%,$(pkgdb_CXXFLAGS) $(CXXFLAGS)))
pkgdb_PC_CFLAGS += $(filter -D%,$(pkgdb_CXXFLAGS) $(CXXFLAGS))
pkgdb_PC_CFLAGS += $(patsubst %,-isystem %,\
                                $(call get_include_dirs,$(boost_CFLAGS)))
pkgdb_PC_CFLAGS += -include $(nix_INCDIR)/nix/config.h
endif  # ifeq (,$(pkgdb_PC_CFLAGS))

pkgdb_PC_LIBS ?= -L$(nix_LIBDIR) -lnixfetchers


# Generate `pkg-config' file.
# ---------------------------
# The `PC_CFLAGS' and `PC_LIBS' variables carry flags that are not covered by
# `nlohmann_json`, `argparse`, `sqlite3pp`, `sqlite`, and `nix{main,cmd,expr}`
# `Requires' handling.
# This amounts to handling `boost', `libnixfetchers', forcing
# the inclusion of the `nix' `config.h' header, and some additional CPP vars.
# For `nix'
lib/pkgconfig/pkgdb.pc: $(lastword $(MAKEFILE_LIST)) $(DEPFILES)
lib/pkgconfig/pkgdb.pc: $(pkgdb_ROOT)/version
	$(MKDIR_P) $(@D);
	{                                                                         \
	  echo 'prefix=$(PREFIX)';                                                \
	  echo 'exec_prefix=$${prefix}';                                          \
	  echo 'includedir=$${prefix}/include';                                   \
	  echo 'libdir=$${prefix}/lib';                                           \
	  echo 'Name: Flox PkgDb';                                                \
	  echo 'Description: CRUD operations for `nix` package metadata.';        \
	  echo 'Version: $(VERSION)';                                             \
	  printf 'Requires: nlohmann_json argparse sqlite3pp sqlite3 nix-main ';  \
	  echo   'nix-cmd nix-expr';                                              \
	  echo 'Cflags: -I$${includedir} $(pkgdb_PC_CFLAGS)';                     \
	  echo 'Libs: -L$${libdir} -lpkgdb $(pkgdb_PC_LIBS)';                     \
	} > $@

CLEANFILES += lib/pkgconfig/pkgdb.pc

install-lib: $(LIBDIR)/pkgconfig/pkgdb.pc


# ---------------------------------------------------------------------------- #

endif  # ifndef __PKGDB_MK

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
