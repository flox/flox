# ============================================================================ #
#
# Adds Compilation Database config files to support various
# Language Server Protocol tools.
#
#
# ---------------------------------------------------------------------------- #

ifndef _MK_CCLS

_MK_CCLS = 1

# ---------------------------------------------------------------------------- #

ifndef MK_DIR
MK_DIR := $(patsubst %/,%,$(dir $(abspath $(lastword $(MAKEFILE_LIST)))))
endif  # ifndef MK_DIR

# ---------------------------------------------------------------------------- #

include $(MK_DIR)/deps.mk

# ---------------------------------------------------------------------------- #

CAT ?= cat
TR  ?= tr
SED ?= sed


# ---------------------------------------------------------------------------- #

.PHONY: ccls
ccls: ../.ccls

../.ccls: FORCE
	echo 'clang' > "$@";
	{                                                                       \
	  echo "$(CXXFLAGS) $(sqlite3_CFLAGS) $(nljson_CFLAGS) $(nix_CFLAGS)";  \
	  echo "$(sql_builder_CXXFLAGS) $(argparse_CFLAGS)";                    \
	  if [[ -n "$(NIX_CC)" ]]; then                                         \
	    $(CAT) "$(NIX_CC)/nix-support/libc-cflags";                         \
	    $(CAT) "$(NIX_CC)/nix-support/libcxx-cxxflags";                     \
	  fi;                                                                   \
	}|$(TR) ' ' '\n'                                                        \
	 |$(SED) -e 's/-std=/%cpp -std=/' -e "s/'//g"                           \
	         -e 's,-I\([^/]\),-Icpp/\1,' >> "$@";


# ---------------------------------------------------------------------------- #

endif  # ifndef _MK_CCLS


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
