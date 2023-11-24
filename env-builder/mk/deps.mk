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

pkgdb_CFLAGS ?=
# $(shell $(PKG_CONFIG) --cflags pkgdb)
pkgdb_CFLAGS := $(pkgdb_CFLAGS)


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
