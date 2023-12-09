# ============================================================================ #
#
# @file build-aux/mk/clean.mk
#
# @brief Provides recipes for cleaning up the build environment.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_CLEAN
__MK_CLEAN = 1

# ---------------------------------------------------------------------------- #

ifeq (,$(MK_DIR))
$(error "$(lastword $(MAKEFILE_LIST)): MK_DIR is not set")
endif  # ifeq (,$(MK_DIR))

include $(MK_DIR)/utils.mk
include $(MK_DIR)/files.mk

# ---------------------------------------------------------------------------- #

#: Remove build artifacts
clean: FORCE
	-$(RM) $(CLEANFILES);
	-$(RM) -r $(CLEANDIRS);
	-$(RM) result;
	-$(RM) **/gmon.out gmon.out **/*.log *.log;
	-$(RM) **/*.gcno *.gcno **/*.gcda *.gcda **/*.gcov *.gcov;


#: Remove build artifacts and metadata files
fullclean: clean
	-$(RM) $(FULLCLEANFILES);
	-$(RM) -r $(FULLCLEANDIRS);


# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_CLEAN

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
