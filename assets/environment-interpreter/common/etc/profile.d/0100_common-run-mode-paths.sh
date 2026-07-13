# shellcheck shell=bash

# ============================================================================ #
#
# Setup common paths needed for `run` mode.
#
# ---------------------------------------------------------------------------- #

# A trailing ':' makes info(1) and emacs append the default (compiled-in)
# search path, so system manuals remain visible. Unlike MANPATH, only a
# trailing separator is special in INFOPATH ("Other Info Directories",
# Texinfo manual). Don't append another ':' if a marker is already present,
# so nested activations don't accumulate empty entries.
case "${INFOPATH:-}" in
  "") INFOPATH="$FLOX_ENV/share/info:" ;;
  *:) INFOPATH="$FLOX_ENV/share/info:$INFOPATH" ;;
  *) INFOPATH="$FLOX_ENV/share/info:$INFOPATH:" ;;
esac
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

export INFOPATH XDG_DATA_DIRS

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
