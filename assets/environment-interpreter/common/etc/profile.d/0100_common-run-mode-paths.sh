# shellcheck shell=bash

# ============================================================================ #
#
# Setup common paths needed for `run` mode.
#
# ---------------------------------------------------------------------------- #

# A trailing ':' makes info(1) append its default (compiled-in) search path,
# so system manuals remain visible. Unlike MANPATH, only a trailing colon is
# special in INFOPATH; see "Other Info Directories" in the Texinfo manual.
INFOPATH="$FLOX_ENV/share/info${INFOPATH:+:$INFOPATH}:"
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

export INFOPATH XDG_DATA_DIRS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
