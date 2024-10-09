# shellcheck shell=bash
export _coreutils="@coreutils@"
export _gnused="@gnused@"
export _fd="@fd@"
export _findutils="@findutils@"
export _ldconfig="@ldconfig@"
export _nawk="@nawk@"
# ============================================================================ #
#
# Setup CUDA
#
# ---------------------------------------------------------------------------- #

activate_cuda() {
  # Strip a trailing or lone slash so that we can construct it later.
  local fhs_root_prefix="${1%/}"
  # Path to ldconfig that can be substituted for testing.
  local ldconfig_bin="$2"
  # Pattern of libraries that we support.
  local lib_pattern="^lib(cuda|nvidia|dxcore).*\.so.*"

  # Only run if _FLOX_ENV_CUDA_DETECTION is set
  if [[ "${_FLOX_ENV_CUDA_DETECTION:-}" != 1 ]]; then
    return 0
  fi

  # Skip when not on Linux
  if [[ "$ldconfig_bin" == "__LINUX_ONLY__" || ! -f "$ldconfig_bin" || ! -x "$ldconfig_bin" ]]; then
    return 0
  fi

  # Skip when no Nvidia device
  if ! ("$_findutils/bin/find" "${fhs_root_prefix}/dev" -maxdepth 1 -iname 'nvidia*' -o -iname dxg | read -r); then
    return 0
  fi

  # Use system library cache.
  SYSTEM_LIBS=$("$ldconfig_bin" --print-cache -C /etc/ld.so.cache 2> /dev/null \
    | "$_nawk/bin/nawk" "\$1 ~ /${lib_pattern}/ { print \$4 }")

  # Fallback for NixOS.
  if [ -z "$SYSTEM_LIBS" ]; then
    # LD_AUDIT workaround for Linux: https://github.com/flox/flox/issues/1341
    SYSTEM_LIBS=$(LD_AUDIT="" "$_fd/bin/fd" "$lib_pattern" "${fhs_root_prefix}/run/opengl-driver" 2> /dev/null)
  fi

  # No matching libs from either results.
  if [ -z "$SYSTEM_LIBS" ]; then
    return 0
  fi

  LIB_DIR="$("$_coreutils/bin/realpath" --no-symlinks "${FLOX_ENV}/../../lib")"
  "$_coreutils/bin/mkdir" -p "$LIB_DIR"

  echo "$SYSTEM_LIBS" | "$_findutils/bin/xargs" "$_coreutils/bin/ln" \
    --symbolic \
    --force \
    --target-directory="$LIB_DIR"

  export FLOX_ENV_LIB_DIRS="$FLOX_ENV_LIB_DIRS":"$LIB_DIR"
}

activate_cuda "/" "$_ldconfig"

# ---------------------------------------------------------------------------- #
#
#
#
# ============================================================================ #
