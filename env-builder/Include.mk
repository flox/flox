# ============================================================================ #
#
# @file env-builder/Include.mk
#
# @brief Provides a set of Makefile variables for use in `env-builder'.
#
# These populate settings needed by this project's `Makefile' and the project
# root `Makefile'.
#
#
# ---------------------------------------------------------------------------- #

ifndef __ENV_BUILDER_MK
__ENV_BUILDER_MK = 1

# ---------------------------------------------------------------------------- #

MAKEFILE_DIR  ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
REPO_ROOT     ?= $(patsubst %/,%,$(dir $(MAKEFILE_DIR)))
REPO_ROOT     := $(REPO_ROOT)
BUILD_AUX_DIR ?= $(REPO_ROOT)/build-aux
BUILD_AUX_DIR := $(BUILD_AUX_DIR)
MK_DIR        ?= $(BUILD_AUX_DIR)/mk
MK_DIR        := $(MK_DIR)

ifeq (,$(wildcard $(REPO_ROOT)/env-builder/))
$(error "Unable to locate repository root")
endif

ENV_BUILDER_ROOT ?= $(REPO_ROOT)/env-builder
ENV_BUILDER_ROOT := $(ENV_BUILDER_ROOT)


# ---------------------------------------------------------------------------- #

include $(MK_DIR)/platform.mk
include $(MK_DIR)/lib.mk
include $(MK_DIR)/files.mk

# ---------------------------------------------------------------------------- #

# Initialize project variables.
$(eval $(call def_cxx_project,env_builder,env-builder))


# ---------------------------------------------------------------------------- #

env_builder_LIBS = lib/libenv-builder$(libExt)
env_builder_BINS = bin/env-builder

env_builder_HEADERS = $(call rwildcard,$(ENV_BUILDER_ROOT)include,*.hh)
env_builder_SRCS    = $(call rwildcard,$(ENV_BUILDER_ROOT)include,*.cc)


# ---------------------------------------------------------------------------- #

endif  # ifndef __ENV_BUILDER_MK

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
