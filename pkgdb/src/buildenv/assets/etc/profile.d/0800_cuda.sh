# shellcheck shell=bash
export _coreutils="@coreutils@"
export _gnused="@gnused@"
export _findutils="@findutils@"
# ============================================================================ #
#
# Setup CUDA
#
# ---------------------------------------------------------------------------- #

# Only run if FLOX_FEATURES_ENV_ENABLE_CUDA feature flag is set
activate_cuda(){
  if [[ "$FLOX_FEATURES_ENV_ENABLE_CUDA" != 1 ]]; then
    return 0
  fi

  if ! ( "$_findutils/bin/find" /dev -maxdepth 1 -iname 'nvidia*' -o -iname dxg | read -r ;); then
    return 0
  fi

  LIB_DIR="$("$_coreutils/bin/realpath" --no-symlinks "$FLOX_ENV/../../lib")"
  "$_coreutils/bin/mkdir" -p "$LIB_DIR"
  for target in libcuda.so.1 libcuda.so libdxcore.so ; do
      "$_findutils/bin/find" /run/opengl-drivers /lib /usr/lib /usr/local/lib \
          -name "$target" \
          -exec "$_coreutils/bin/ln" -sf {} "$LIB_DIR"/"$target" \; \
          -quit 2>/dev/null
  done
  export FLOX_ENV_LIB_DIRS="$FLOX_ENV_LIB_DIRS":"$LIB_DIR"
}

activate_cuda

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
