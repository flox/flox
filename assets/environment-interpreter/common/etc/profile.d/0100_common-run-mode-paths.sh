# shellcheck shell=bash

# ============================================================================ #
#
# Setup common paths needed for `run` mode.
#
# ---------------------------------------------------------------------------- #

# A trailing ':' makes info(1) and emacs append the default (compiled-in)
# search path, so system manuals remain visible ("Other Info Directories",
# Texinfo manual). When INFOPATH is unset or empty, the expansion below
# leaves exactly that trailing ':'. An existing value is preserved verbatim,
# including the user's choice to include or exclude the default path.
INFOPATH="$FLOX_ENV/share/info:${INFOPATH:-}"
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

export INFOPATH XDG_DATA_DIRS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
