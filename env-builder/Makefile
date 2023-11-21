# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

ROOT_DIR := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
export ROOT_DIR

# ---------------------------------------------------------------------------- #

include mk/config.mk

# ---------------------------------------------------------------------------- #

BINS    =  env-builder
LIBS    =  libenvbuilder
HEADERS =  $(wildcard include/*.hh include/flox/*.hh include/nix/*.hh)


# ---------------------------------------------------------------------------- #

include mk/deps.mk
include mk/lib.mk
include mk/ccls.mk
include mk/clean.mk
include mk/check.mk
include mk/docs.mk
include src/Include.mk


# ---------------------------------------------------------------------------- #

CXX     ?= c++
MKDIR   ?= mkdir
MKDIR_P ?= $(MKDIR) -p
CP      ?= cp


# ---------------------------------------------------------------------------- #

CPPFLAGS ?=
LDLIBS   ?=

CXXFLAGS ?=
CXXFLAGS += -Iinclude
CXXFLAGS += $(nix_CFLAGS) $(sqlite3pp_CFLAGS)

LDFLAGS ?=
LDFLAGS += -Wl,-rpath,$$ORIGIN/../lib
# LDFLAGS += -Wl,--enable-new-dtags '-Wl,-rpath,$$ORIGIN/../lib'
LDFLAGS += $(nix_LDFLAGS)


bin_CPPFLAGS ?=
bin_CXXFLAGS ?=
bin_LDFLAGS  ?= -Llib
bin_LDLIBS   ?=

lib_CPPFLAGS ?=
lib_CXXFLAGS ?= -shared -fPIC
lib_LDFLAGS  ?= -shared -fPIC -Wl
# lib_LDFLAGS  ?= -shared -fPIC -Wl,--no-undefined
lib_LDLIBS   ?=


# ---------------------------------------------------------------------------- #

ifneq (,$(DEBUG))
CXXFLAGS += -ggdb3 -pg -fno-omit-frame-pointer
LDFLAGS  += -ggdb3 -pg -fno-omit-frame-pointer
endif

ifneq (,$(COV))
CXXFLAGS += --coverage -fprofile-arcs -ftest-coverage
LDFLAGS  += --coverage -fprofile-arcs -ftest-coverage
endif


# ---------------------------------------------------------------------------- #

ifneq (,$(SEMVER))
CXXFLAGS += -DSEMVER_PATH='$(SEMVER)'
endif


# ---------------------------------------------------------------------------- #

include mk/gen-targets.mk
# This needs to come after we run our templates.
include mk/install.mk


# ---------------------------------------------------------------------------- #

.PHONY: all
.DEFAULT_GOAL = all

all: bin lib include tests


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
