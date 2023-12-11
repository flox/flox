# ============================================================================ #
#
# @file build-aux/mk/files.mk
#
# @brief Provides variables used to aggregate files/target lists.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_FILES
__MK_FILES = 1

# ---------------------------------------------------------------------------- #

MK_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
MK_DIR := $(abspath $(MK_DIR))

# ---------------------------------------------------------------------------- #

include $(MK_DIR)/lib.mk

# ---------------------------------------------------------------------------- #

# Repo directories.

BUILD_AUX_DIR ?= $(patsubst %/,%,$(MK_DIR))
BUILD_AUX_DIR := $(abspath $(BUILD_AUX_DIR))

REPO_ROOT ?= $(patsubst %/,%,$(dir $(BUILD_AUX_DIR)))
REPO_ROOT := $(abspath $(REPO_ROOT))


# ---------------------------------------------------------------------------- #

CXX_PROJECTS  =
RUST_PROJECTS =
PROJECTS      = $(CXX_PROJECTS) $(RUST_PROJECTS)


# ---------------------------------------------------------------------------- #

ifeq (,$(REPO_ROOT))
$(error "$(lastword $(MAKEFILE_LIST)): REPO_ROOT is not set")
endif  # ifeq (,$(REPO_ROOT))


# ---------------------------------------------------------------------------- #

define def_cxx_project_files =
CXX_PROJECTS        += $(1)
$(1)_ROOT           ?= $$(REPO_ROOT)/$(if $(2),$(2),$(1))
$(1)_LIBS           =
$(1)_BINS           =
$(1)_TEST_BINS      =
$(1)_TESTS          =
$(1)_HEADERS        =
$(1)_TEST_SRCS      =
$(1)_SRCS           =
$(1)_ALL_SRCS       =  $$($(1)_SRCS) $$($(1)_TEST_SRCS)
$(1)_CLEANFILES     =  $$($(1)_ALL_SRCS:.cc=.o)
$(1)_CLEANFILES     += $$($(1)_LIBS) $$($(1)_BINS) $$($(1)_TEST_BINS)
$(1)_CLEANDIRS      =
$(1)_FULLCLEANFILES =
$(1)_FULLCLEANDIRS  =
endef  # define def_cxx_project_files


# ---------------------------------------------------------------------------- #

LIBS      = $(foreach proj,$(CXX_PROJECTS),$($(proj)_LIBS))
BINS      = $(foreach proj,$(CXX_PROJECTS),$($(proj)_BINS))
TEST_BINS = $(foreach proj,$(CXX_PROJECTS),$($(proj)_TEST_BINS))
TESTS     = $(foreach proj,$(CXX_PROJECTS),$($(proj)_TESTS))
HEADERS   = $(foreach proj,$(CXX_PROJECTS),$($(proj)_HEADERS))
SRCS      = $(foreach proj,$(CXX_PROJECTS),$($(proj)_SRCS))


# ---------------------------------------------------------------------------- #

CLEANFILES     = $(foreach proj,$(CXX_PROJECTS),$($(proj)_CLEANFILES))
CLEANDIRS      = $(foreach proj,$(CXX_PROJECTS),$($(proj)_CLEANDIRS))
FULLCLEANFILES = $(foreach proj,$(CXX_PROJECTS),$($(proj)_FULLCLEANFILES))
FULLCLEANDIRS  = $(foreach proj,$(CXX_PROJECTS),$($(proj)_FULLCLEANDIRS))

# Some sane defaults to clean.
CLEANFILES += $(call rwildcard,$(REPO_ROOT),result *.log *~)
CLEANFILES += $(call rwildcard,$(REPO_ROOT),*.gcno *.gcda *.gcov gmon.out)


# ---------------------------------------------------------------------------- #

# Files which effect dependencies, external inputs, and `*FLAGS' values.

DEPFILES =
DEPFILES += $(REPO_ROOT)/flake.nix
DEPFILES += $(REPO_ROOT)/flake.lock
DEPFILES += $(REPO_ROOT)/pkgs/nlohmann_json.nix
DEPFILES += $(REPO_ROOT)/pkgs/nix/default.nix
DEPFILES += $(REPO_ROOT)/pkgs/flox-pkgdb/default.nix
DEPFILES += $(REPO_ROOT)/pkgs/flox-env-builder/default.nix
DEPFILES += $(REPO_ROOT)/shells/flox/default.nix
DEPFILES += $(MK_DIR)/deps.mk
# Only include `config.mk' if it exists.
DEPFILES += $(if $(wildcard $(REPO_ROOT)/config.mk),$(REPO_ROOT)/config.mk)


# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_FILES

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
