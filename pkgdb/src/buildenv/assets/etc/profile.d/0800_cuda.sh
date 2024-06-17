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
  SYSTEM_LIB_DIR=$("$_findutils/bin/find" \
	  /run/opengl-drivers /lib64 /lib /usr/lib64 /usr/lib /usr/local/lib64 /usr/local/lib \
          -name libcuda.so.1 \
	  -execdir pwd \; -quit 2>/dev/null )

  if [ -z "$SYSTEM_LIB_DIR" ]; then
    return 0
  fi

  "$_coreutils/bin/mkdir" -p "$LIB_DIR"
  (
    shopt -s nullglob
    "$_coreutils/bin/ln" -sft "$LIB_DIR"/. \
      "$SYSTEM_LIB_DIR"/libcuda*.so*   \
      "$SYSTEM_LIB_DIR"/libnvidia*.so* \
      "$SYSTEM_LIB_DIR"/libdxcore*.so*
  )
  export FLOX_ENV_LIB_DIRS="$FLOX_ENV_LIB_DIRS":"$LIB_DIR"
}

activate_cuda

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
