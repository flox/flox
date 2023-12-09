# ============================================================================ #
#
# @file build-aux/mk/flags.mk
#
# @brief Provides variables used to aggregate flag/option lists.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_FLAGS
__MK_FLAGS = 1

# ---------------------------------------------------------------------------- #

ifeq (,$(MK_DIR))
$(error "$(lastword $(MAKEFILE_LIST)): MK_DIR is not set")
endif  # ifeq (,$(MK_DIR))

include $(MK_DIR)/files.mk
include $(MK_DIR)/platform.mk

# ---------------------------------------------------------------------------- #

define def_cxx_project_flags =
$(1)_CXXFLAGS =
$(1)_LDFLAGS  =
endef  # define def_cxx_project_flags


# ---------------------------------------------------------------------------- #

def_cxx_project =  $(call def_cxx_project_files,$(*))
def_cxx_project += $(call def_cxx_project_flags,$(*))


# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_FLAGS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
