# shellcheck shell=bash
export _coreutils="@coreutils@"
export _gnused="@gnused@"
export _findutils="@findutils@"
# ============================================================================ #
#
# Setup CUDA
#
# ---------------------------------------------------------------------------- #

# Only run if _FLOX_ENV_CUDA_DETECTION is set
activate_cuda(){
  # Strip any trailing slash so that we can construct it later.
  local fhs_root_prefix="${1%/:-}"

  if [[ "${_FLOX_ENV_CUDA_DETECTION:-}" != 1 ]]; then
    return 0
  fi

  if ! ( "$_findutils/bin/find" "${fhs_root_prefix}/dev" -maxdepth 1 -iname 'nvidia*' -o -iname dxg | read -r ;); then
    return 0
  fi

  LIB_DIR="$("$_coreutils/bin/realpath" --no-symlinks "$FLOX_ENV/../../lib")"
  SYSTEM_LIB_DIR=$("$_findutils/bin/find" \
    "${fhs_root_prefix}/run/opengl-drivers" \
    "${fhs_root_prefix}/lib64" \
    "${fhs_root_prefix}/lib" \
    "${fhs_root_prefix}/usr/lib64" \
    "${fhs_root_prefix}/usr/lib" \
    "${fhs_root_prefix}/usr/local/lib64" \
    "${fhs_root_prefix}/usr/local/lib" \
    -name libcuda.so.1 \
    -printf '%h\n' -quit 2>/dev/null || true)

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

activate_cuda "/"

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
