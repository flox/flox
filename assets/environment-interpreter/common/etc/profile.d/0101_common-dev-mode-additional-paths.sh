# shellcheck shell=bash
_uname="@coreutils@/bin/uname"
_ld_floxlib_so="@ld_floxlib@/lib/ld-floxlib.so"

# ============================================================================ #
#
# Setup common paths needed for `dev` mode additional to those in `run` mode.
#
# ---------------------------------------------------------------------------- #

CPATH="$FLOX_ENV/include${CPATH:+:$CPATH}"
LIBRARY_PATH="$FLOX_ENV/lib${LIBRARY_PATH:+:$LIBRARY_PATH}"
PKG_CONFIG_PATH="$FLOX_ENV/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
PKG_CONFIG_PATH="$FLOX_ENV/lib/pkgconfig:$PKG_CONFIG_PATH"
ACLOCAL_PATH="$FLOX_ENV/share/aclocal${ACLOCAL_PATH:+:$ACLOCAL_PATH}"

export \
  CPATH \
  LIBRARY_PATH \
  PKG_CONFIG_PATH \
  ACLOCAL_PATH \
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
        # The runtime loader reserves a static amount of memory for thread local
        # storage (TLS).
        # When using LD_AUDIT in combination with libraries such as jemalloc that use TLS,
        # the default reservation is exceeded and the program fails to start.
        # Use GLIBC_TUNABLES to increase the reservation.
        # The default reservation is 512.
        # Requirements for some programs we've run into the issue with:
        # On x86_64-linux:
        # redis-server: 1441
        # biome: 1417
        # fd: 1417
        # On aarch64-linux:
        # redis-server: 1777
        # biome: 1553
        # fd: 1561
        # See https://sourceware.org/bugzilla/show_bug.cgi?id=31991
        export GLIBC_TUNABLES=glibc.rtld.optional_static_tls=25000
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
