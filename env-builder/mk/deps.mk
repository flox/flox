# ============================================================================ #
#
# Set `<DEP>_(CFLAGS|LDFLAGS|*)' style variables.
#
#
# ---------------------------------------------------------------------------- #

ifndef _MK_DEPS

_MK_DEPS = 1

# ---------------------------------------------------------------------------- #

ifndef MK_DIR
MK_DIR :=                                                                    \
  $(patsubst $(CURDIR)/%/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
endif  # ifndef MK_DIR

# ---------------------------------------------------------------------------- #

PKG_CONFIG ?= pkg-config

# ---------------------------------------------------------------------------- #

nljson_CFLAGS ?= $(shell $(PKG_CONFIG) --cflags nlohmann_json)
nljson_CFLAGS := $(nljson_CFLAGS)


# ---------------------------------------------------------------------------- #

# TODO: This block is an absolute shit-show.
# There's a draft branch that uses `autoconf' and `automake' which will clean
# this up; but for now this is going to allow us to make incremental progress
# on fixing the `devShell'.

pkgdb_CFLAGS ?= $(shell $(PKG_CONFIG) --cflags pkgdb||:)
pkgdb_CFLAGS := $(pkgdb_CFLAGS)
ifeq (,$(pkgdb_CFLAGS))
pkgdb_CFLAGS := -I$(MK_DIR)/../../pkgdb/include
endif  # ifeq (,$(pkgdb_CFLAGS))

pkgdb_LIBS   ?= $(shell $(PKG_CONFIG) --libs pkgdb||:)
pkgdb_LIBS   := $(pkgdb_LIBS)
ifeq (,$(pkgdb_LIBS))
pkgdb_LIBDIR ?= $(abspath $(MK_DIR)/../../pkgdb/lib)
pkgdb_LIBDIR := $(pkgdb_LIBDIR)
pkgdb_LIBS   += -L$(pkgdb_LIBDIR)
ifeq ($(shell $(UNAME) -s),Linux)
pkgdb_LIBS += -Wl,-rpath,$(pkgdb_LIBDIR)
libExt := .so
else # Darwin
pkgdb_LIBS += -rpath $(pkgdb_LIBDIR)
libExt := .dylib
endif  # ifeq ($(shell $(UNAME) -s),Linux)
pkgdb_LIBS += -lflox-pkgdb
pkgdb_LIBS := $(pkgdb_LIBS)
$(MK_DIR)/../bin/flox-env-builder: $(MK_DIR)/../../pkgdb/lib/libflox-pkgdb$(libExt)
$(MK_DIR)/../../pkgdb/lib/libflox-pkgdb$(libExt):
	$(MAKE) -C $(MK_DIR)/../../pkgdb lib/libpkgdb$(libExt)
endif  # ifeq (,$(pkgdb_LIBS))


# ---------------------------------------------------------------------------- #

boost_CFLAGS ?=
boost_CFLAGS := $(boost_CFLAGS)


# ---------------------------------------------------------------------------- #

ifndef nix_CFLAGS
nix_INCDIR ?= $(shell $(PKG_CONFIG) --variable=includedir nix-cmd)

ifndef nix_CPPFLAGS
nix_CPPFLAGS =  $(boost_CPPFLAGS)
nix_CPPFLAGS += -isystem '$(nix_INCDIR)' -include $(nix_INCDIR)/nix/config.h
endif  # ifndef nix_CPPFLAGS

nix_CFLAGS =  $(nix_CPPFLAGS)
nix_CFLAGS += $(shell $(PKG_CONFIG) --cflags nix-main nix-cmd nix-expr)
endif  # ifndef nix_CFLAGS
nix_CFLAGS := $(nix_CFLAGS)

ifndef nix_LDFLAGS
nix_LDFLAGS =                                                        \
  $(shell $(PKG_CONFIG) --libs nix-main nix-cmd nix-expr nix-store)
nix_LDFLAGS += -lnixfetchers
# For `libnixstore.so'
nix_LDFLAGS += $(sqlite3_LDFLAGS)
endif  # infndef nix_LDFLAGS
nix_LDFLAGS := $(nix_LDFLAGS)


# ---------------------------------------------------------------------------- #

endif  # ifndep _MK_DEPS


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
