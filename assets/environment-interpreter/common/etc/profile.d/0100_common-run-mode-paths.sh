# shellcheck shell=bash

# ============================================================================ #
#
# Setup common paths needed for `run` mode.
#
# ---------------------------------------------------------------------------- #

INFOPATH="$FLOX_ENV/share/info${INFOPATH:+:$INFOPATH}"
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

export INFOPATH XDG_DATA_DIRS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
