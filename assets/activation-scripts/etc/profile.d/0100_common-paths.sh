# shellcheck shell=bash
export _coreutils="@coreutils@"
# ============================================================================ #
#
# Setup common paths.
#
# ---------------------------------------------------------------------------- #

INFOPATH="$FLOX_ENV/share/info${INFOPATH:+:$INFOPATH}"
CPATH="$FLOX_ENV/include${CPATH:+:$CPATH}"
LIBRARY_PATH="$FLOX_ENV/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
PKG_CONFIG_PATH="$FLOX_ENV/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
PKG_CONFIG_PATH="$FLOX_ENV/lib/pkgconfig:$PKG_CONFIG_PATH"
ACLOCAL_PATH="$FLOX_ENV/share/aclocal${ACLOCAL_PATH:+:$ACLOCAL_PATH}"
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}"

export \
  INFOPATH \
  CPATH \
  LIBRARY_PATH \
  PKG_CONFIG_PATH \
  ACLOCAL_PATH \
  XDG_DATA_DIRS \
  ;

# ---------------------------------------------------------------------------- #

if [ -n "${FLOX_ENV_LIB_DIRS:-}" ]; then
  case "$($_coreutils/bin/uname -s)" in
    Linux*)
      # N.B. ld-floxlib.so makes use of FLOX_ENV_LIB_DIRS directly.
      LD_FLOXLIB="${LD_FLOXLIB:-@ld-floxlib@}"
      if [ -z "${FLOX_NOSET_LD_AUDIT:-}" ] && [ -e "$LD_FLOXLIB" ]; then
        LD_AUDIT="$LD_FLOXLIB"
        export LD_AUDIT
      fi
      ;;
    Darwin*)
      if [ -z "${FLOX_NOSET_DYLD_FALLBACK_LIBRARY_PATH:-}" ]; then
        DYLD_FALLBACK_LIBRARY_PATH="$FLOX_ENV_LIB_DIRS:${DYLD_FALLBACK_LIBRARY_PATH:-/usr/local/lib:/usr/lib}"
        export DYLD_FALLBACK_LIBRARY_PATH
      fi
      ;;
  esac
fi

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
