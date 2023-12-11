# ============================================================================ #
#
# @file build-aux/mk/platform.mk
#
# @brief Sets platform specific variables.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_PLATFORM
__MK_PLATFORM = 1

# ---------------------------------------------------------------------------- #

MK_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
MK_DIR := $(abspath $(MK_DIR))

# ---------------------------------------------------------------------------- #

include $(MK_DIR)/utils.mk

# ---------------------------------------------------------------------------- #

# Detect OS and Set Shared Library Extension
# ------------------------------------------

OS ?= $(shell $(UNAME))
OS := $(OS)
ifndef libExt
ifeq (Linux,$(OS))
libExt ?= .so
else
libExt ?= .dylib
endif  # ifeq (Linux,$(OS))
endif  # ifndef libExt


# ---------------------------------------------------------------------------- #

# Detect the C++ compiler toolchain
# ---------------------------------

ifndef TOOLCHAIN

ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'gcc'||:)"
TOOLCHAIN = gcc
else ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'clang'||:)"
TOOLCHAIN = clang
else
$(error "Unable to detect C++ compiler toolchain for CXX: $(CXX)")
endif  # ifneq "" "$(shell $(CXX) --version|$(GREP) -i 'gcc'||:)"

else  # ifndef TOOLCHAIN

# If the user set TOOLCHAIN, ensure that it is valid.
ifeq "" "$(filter gcc clang,$(TOOLCHAIN))"
$(error "Invalid C++ compiler toolchain: $(TOOLCHAIN)")
endif  # ifeq "" "$(filter gcc clang,$(TOOLCHAIN))"

endif  # ifndef TOOLCHAIN


# ---------------------------------------------------------------------------- #

ifeq (linux,$(OS))
RELATIVE_RPATH_FLAG = -Wl,--enable-new-dtags '-Wl,-rpath,$$$$ORIGIN/../lib'
# Set/append the executable's `RUNPATH' to the given path.
set_rpath = -Wl,--enable-new-dtags '-Wl,--rpath,$$(notdir $(1))'
# Set/append the executable's `SONAME'.
set_binary_name = '-Wl,-soname,$(1)'
else  # Darwin
RELATIVE_RPATH_FLAG = -rpath @executable_path/../lib
# Set/append the executable's `RPATH' to the given path.
set_rpath  = -rpath $(1)
# Set/append the executable's _install name_ ( OSX `SONAME' equivalent ).
set_binary_name =$(strip
  $$(if $$(filter $(1),$$(notdir $(1))),\
  $$(error Binary name must be a basename, but got: '$(1)'),\
  -install_name '@rpath/$(1)'))
endif  # ifeq (linux,$(OS))


# ---------------------------------------------------------------------------- #

endif # ifndef __MK_PLATFORM

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
