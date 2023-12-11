# ============================================================================ #
#
# @file build-aux/mk/nix.mk
#
# @brief Provides variables and helper functions for building with Nix.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_NIX
__MK_NIX = 1

# ---------------------------------------------------------------------------- #

MK_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
MK_DIR := $(abspath $(MK_DIR))

# ---------------------------------------------------------------------------- #

include $(MK_DIR)/utils.mk
include $(MK_DIR)/lib.mk

# ---------------------------------------------------------------------------- #

# Extract compiler flags from Nix `CC' wrapper.
NIX_CC_WRAPPER_CFLAGS   ?=
NIX_CC_WRAPPER_CXXFLAGS ?=
ifneq (,$(NIX_CC))
NIX_CC_WRAPPER_CFLAGS   = $(file < $(NIX_CC)/nix-support/libc-cflags)
NIX_CC_WRAPPER_CXXFLAGS = $(file < $(NIX_CC)/nix-support/libcxx-cxxflags)
endif # ifneq (,$(NIX_CC))


# ---------------------------------------------------------------------------- #

# Get system include paths from C++ compiler.
# Filter out framework directory, e.g.
# `.*/Library/Frameworks' ( framework directory )
ifeq (,$(NIX_CXX_SYSTEM_INCLUDES))
NIX_CXX_SYSTEM_INCLUDES = $(strip $(patsubst %,-isystem %,\
  $(shell $(CXX) -E -Wp,-v -xc++ /dev/null 2>&1 1>/dev/null              \
            |$(GREP) -v 'framework directory'|$(GREP) '^ /nix/store')))
endif  # ifeq (,$(NIX_CXX_SYSTEM_INCLUDES))



# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_NIX

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
