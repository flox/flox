# ============================================================================ #
#
#
#
# ---------------------------------------------------------------------------- #

SRC_DIR := $(call getMakefileDir)


# ---------------------------------------------------------------------------- #

_BUILT_SRCS := buildenv.nix get-env.sh
_BUILT      := $(patsubst %,$(SRC_DIR)/%.gen.hh,$(_BUILT_SRCS))
_BUILT_SRCS =

BUILT_SRCS       += $(_BUILT)
env_builder_SRCS += $(_BUILT)
env_builder_SRCS += $(wildcard $(SRC_DIR)/*.cc)
env_builder_LDLIBS += -lflox-pkgdb
libenvbuilder_LDLIBS += -lsqlite3


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
