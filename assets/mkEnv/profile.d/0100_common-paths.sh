# ============================================================================ #
#
# Setup common paths.
#
# ---------------------------------------------------------------------------- #

PATH="$FLOX_ENV/bin:$FLOX_ENV/sbin${PATH:+:$PATH}";
FPATH="$FLOX_ENV/share/zsh/vendor-completions${FPATH:+:$FPATH}";
FPATH="$FLOX_ENV/share/zsh/site-functions:$FPATH";
MANPATH="$FLOX_ENV/share/man${MANPATH:+:$MANPATH}";
INFOPATH="$FLOX_ENV/share/info${INFOPATH:+:$INFOPATH}";
CPATH="$FLOX_ENV/include${CPATH:+:$CPATH}";
LIBRARY_PATH="$FLOX_ENV/lib${LIBRARY_PATH:+:$LIBRARY_PATH}";
PKG_CONFIG_PATH="$FLOX_ENV/share/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}";
PKG_CONFIG_PATH="$FLOX_ENV/lib/pkgconfig:$PKG_CONFIG_PATH";
ACLOCAL_PATH="$FLOX_ENV/share/aclocal${ACLOCAL_PATH:+:$ACLOCAL_PATH}";
XDG_DATA_DIRS="$FLOX_ENV/share${XDG_DATA_DIRS:+:$XDG_DATA_DIRS}";

export             \
  PATH             \
  FPATH            \
  MANPATH          \
  INFOPATH         \
  CPATH            \
  LIBRARY_PATH     \
  PKG_CONFIG_PATH  \
  ACLOCAL_PATH     \
  XDG_DATA_DIRS    \
;

if [ -z "${FLOX_NOSET_LD_LIBRARY_PATH:-}" ]; then
  LD_LIBRARY_PATH="$FLOX_ENV/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}";
  export LD_LIBRARY_PATH;
fi


# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
