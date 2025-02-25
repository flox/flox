# shellcheck shell=bash
_uname="@coreutils@/bin/uname"
_ld_floxlib_so="@ld_floxlib@/lib/ld-floxlib.so"

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

if [ -n "${FLOX_ENV_DIRS:-}" ]; then
  case "$($_uname -s)" in
    Linux*)
      # N.B. ld-floxlib.so makes use of FLOX_ENV_DIRS directly.
      LD_FLOXLIB="${LD_FLOXLIB:-$_ld_floxlib_so}"
      if [ -z "${FLOX_NOSET_LD_AUDIT:-}" ] && [ -e "$LD_FLOXLIB" ]; then
        LD_AUDIT="$LD_FLOXLIB"
        export LD_AUDIT
      fi
      ;;
    Darwin*)
      if [ -z "${FLOX_NOSET_DYLD_FALLBACK_LIBRARY_PATH:-}" ]; then
        # Calculate a list of FLOX_ENV directories with "/lib" appended.
        _flox_env_lib_dirs=""
        _ifs_orig="$IFS"
        IFS=:
        for dir in $FLOX_ENV_DIRS; do
          _flox_env_lib_dirs="${_flox_env_lib_dirs:+${_flox_env_lib_dirs}:}$dir/lib"
        done
        IFS="$_ifs_orig"
        DYLD_FALLBACK_LIBRARY_PATH="$_flox_env_lib_dirs:${DYLD_FALLBACK_LIBRARY_PATH:-/usr/local/lib:/usr/lib}"
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
