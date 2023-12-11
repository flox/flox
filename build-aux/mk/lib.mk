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

MK_DIR ?= $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
MK_DIR := $(abspath $(MK_DIR))

# ---------------------------------------------------------------------------- #

# Whitespace Character Literals

EMPTY :=
SPACE := $(EMPTY) $(EMPTY)
COMMA := $(EMPTY),$(EMPTY)
define NEWLINE

$(EMPTY)
endef

# ---------------------------------------------------------------------------- #

# rwildcard DIRS..., PATTERNS...
# ------------------------------
# Recursive wildcard.
#   Ex:  $(call rwildcard,src,*.cc *.hh)
rwildcard = $(foreach dir,$(wildcard $(1:=/*)),                \
                          $(call rwildcard,$(dir),$(2))        \
                          $(filter $(subst *,%,$(2)),$(dir)))


# ---------------------------------------------------------------------------- #

# get_flag_args FLAG, ARGS...
# ---------------------------
# Extract the arguments to an option of the form `<FLAG>ARG' or `FLAG ARG` from
# a list of arguments.
# `FLAG' should include any `-' characters ( e.g. `-I' or `--foo' ).
#
# Example:
#   $(call get_flag_args,-std,-Iinclude -std c++2a -std=c++20 -stdc++2b)
#   => c++2a c++2b

# Helper function for `get_flag_args' executed like a _reduce_ or `foldl'.
_get_flag_args = $(strip $(if $(3),\
$(let flag next rest,\
  $(patsubst $(1)%,$(1) %,$(3)),\
  $(if $(filter $(1),$(flag)),\
    $(call _get_flag_args,$(1),$(if $(2),$(2) )$(next),$(rest)),\
    $(call _get_flag_args,$(1),$(2),$(next)$(if $(rest), $(rest))))),\
$(2)))

get_flag_args = $(call _get_flag_args,$(1),,$(2))


# ---------------------------------------------------------------------------- #

# get_opt_args OPT, ARGS...
# -------------------------
# Extract the arguments to an option of the form `OPT=ARG' or `OPT ARG` from
# a list of arguments.
# `OPT' should include any `-' characters ( e.g. `-I' or `--foo' ).
#
# Example:
#   $(call get_opt_args,-std,-Iinclude -std c++2a -std=c++20 -stdc++2b)
#   => c++2a c++20

# Helper function for `get_opt_args' executed like a _reduce_ or `foldl'.
_get_opt_args = $(strip $(if $(3),\
$(let flag next rest,\
  $(patsubst $(1)=%,$(1) %,$(3)),\
  $(if $(filter $(1),$(flag)),\
    $(call _get_opt_args,$(1),$(if $(2),$(2) )$(next),$(rest)),\
    $(call _get_opt_args,$(1),$(2),$(next)$(if $(rest), $(rest))))),\
$(2)))

get_opt_args = $(call _get_opt_args,$(1),,$(2))


# ---------------------------------------------------------------------------- #

# get_include_dirs ARGS...
# -------------------------
# Exctract the directory part of any `-I<DIR>`,`-I <DIR>`, `-isystem <DIR>`,
# or `-isystem<DIR>` flags in `ARGS`.
#
# Example:
#   $(call get_include_dirs,-I/usr/include -isystem /usr/local/include -O2)
#   => /usr/include /usr/local/include

get_include_dirs = $(strip $(call get_flag_args,-I,$(1))        \
                           $(call get_opt_args,-isystem,$(1)))


# ---------------------------------------------------------------------------- #

# get_cc_std ARGS...
# ------------------
# Extract the C/C++ language standard from a set of compiler flags.
get_cc_std = $(lastword $(call get_opt_args,-std,$(1)))


# ---------------------------------------------------------------------------- #

.PHONY: FORCE

# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_LIB

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
