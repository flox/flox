# shellcheck shell=bash
export _coreutils="@coreutils@"
export _gnused="@gnused@"
export _findutils="@findutils@"
export _ldconfig="@ldconfig@"
# ============================================================================ #
#
# Setup CUDA
#
# ---------------------------------------------------------------------------- #

activate_cuda(){
  # Strip a trailing or lone slash so that we can construct it later.
  local fhs_root_prefix="${1%/}"
  # Path to ldconfig that can be substituted for testing.
  local ldconfig_bin="$2"

  # Only run if _FLOX_ENV_CUDA_DETECTION is set
  if [[ "${_FLOX_ENV_CUDA_DETECTION:-}" != 1 ]]; then
    return 0
  fi

  # Skip when not on Linux
  if [[ "$ldconfig_bin" == "__LINUX_ONLY__" || ! -f "$ldconfig_bin" || ! -x "$ldconfig_bin" ]]; then
    return 0
  fi

  # Skip when no Nvidia device
  if ! ( "$_findutils/bin/find" "${fhs_root_prefix}/dev" -maxdepth 1 -iname 'nvidia*' -o -iname dxg | read -r ;); then
    return 0
  fi

  SYSTEM_LIBS=$("$ldconfig_bin" --print-cache -C /etc/ld.so.cache 2>/dev/null \
    | awk '$1 ~ /^lib(cuda|nvidia|dxcore).*\.so.*/ { print $4 }')
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
