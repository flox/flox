# ============================================================================ #
#
# @file build-aux/mk/lib.mk
#
# @brief Provides helper functions used by the Makefiles.
#
# These are often invoked using `$(call FN,ARGS...)'.
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_LIB
__MK_LIB = 1

# ---------------------------------------------------------------------------- #

# rwildcard DIRS, PATTERNS
# ------------------------
# Recursive wildcard.
#   Ex:  $(call rwildcard,src,*.cc *.hh)
rwildcard = $(foreach dir,$(wildcard $(1:=/*)),                \
                          $(call rwildcard,$(dir),$(2))        \
                          $(filter $(subst *,%,$(2)),$(dir)))


# ---------------------------------------------------------------------------- #

# get_include_dirs FLAGS
# ----------------------
# Exctract the directory part of any `-I<DIR>`,`-I <DIR>`, `-isystem <DIR>`,
# or `-isystem<DIR>` flags in `FLAGS`.
#
# Example:
#   $(call get_include_dirs,-I/usr/include -isystem /usr/local/include -O2)
#   => /usr/include /usr/local/include
_get_include_dirs = $(strip $(if $(2),\
$(let flag next rest,\
  $(patsubst -isystem=%,-isystem %,$(patsubst -I%,-I %,$(2))),\
  $(if $(filter -I -isystem,$(flag)),\
    $(call _get_include_dirs,$(if $(1),$(1) )$(next),$(rest)),\
    $(call _get_include_dirs,$(1),$(next)$(if $(rest), $(rest))))),\
$(1)))

get_include_dirs = $(call _get_include_dirs,,$(1))


# ---------------------------------------------------------------------------- #

.PHONY: FORCE

# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_LIB

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
