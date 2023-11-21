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
NIX        ?= nix
JQ         ?= jq

FLAKE_LOCK ?= $(ROOT_DIR)/../flake.lock


# ---------------------------------------------------------------------------- #

getLockedRev = $(shell $(JQ) -r '.nodes["$1"].locked.rev' $(FLAKE_LOCK))

# DO NOT perform eager expansion here.
NIXPKGS_REF ?= github:NixOS/nixpkgs/$(call getLockedRev,nixpkgs)


# ---------------------------------------------------------------------------- #

getNixOutpath = $(shell $(NIX) build --no-link --print-out-paths $1)


# ---------------------------------------------------------------------------- #

nljson_CFLAGS ?= $(shell $(PKG_CONFIG) --cflags nlohmann_json)
nljson_CFLAGS := $(nljson_CFLAGS)


# ---------------------------------------------------------------------------- #

argparse_CFLAGS ?= $(shell $(PKG_CONFIG) --cflags argparse)
argparse_CFLAGS := $(argparse_CFLAGS)


# ---------------------------------------------------------------------------- #

boost_CPPFLAGS ?=                                                    \
  -isystem $(call getNixOutpath,'$(NIXPKGS_REF)#boost.dev')/include
boost_CPPFLAGS := $(boost_CPPFLAGS)

boost_CFLAGS ?= $(boost_CPPFLAGS)
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
