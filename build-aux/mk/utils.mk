# ============================================================================ #
#
# @file build-aux/mk/utils.mk
#
# @brief Sets variables for utilities used by the Makefile and test suite.
#
# This allows these tools to be overridden or passed in with absolute paths
# if desired to avoid modifying `PATH` in the build environment..
#
#
# ---------------------------------------------------------------------------- #

ifndef __MK_UTILS
__MK_UTILS = 1

# ---------------------------------------------------------------------------- #

# Utilities
# ---------
# We use variables to refer to all tools so that we can easily override them
# from the command line.

BEAR       ?= bear
CAT        ?= cat
CP         ?= cp
CXX        ?= c++
DOXYGEN    ?= doxygen
FMT        ?= clang-format
GREP       ?= grep
LN         ?= ln -f
MKDIR      ?= mkdir
MKDIR_P    ?= $(MKDIR) -p
NIX        ?= nix
PKG_CONFIG ?= pkg-config
RM         ?= rm -f
SED        ?= sed
TEE        ?= tee
TEST       ?= test
TIDY       ?= clang-tidy
TOUCH      ?= touch
TR         ?= tr
UNAME      ?= uname


# ---------------------------------------------------------------------------- #

endif  # ifndef __MK_UTILS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
